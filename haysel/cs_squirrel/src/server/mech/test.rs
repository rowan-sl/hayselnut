use std::time::Duration;

use tokio::{select, spawn, time::sleep};

use super::{Config, ConnState, Send};
use crate::{
    env::Env,
    packet::{self, uid::Uid},
};

fn test_conf_env() -> (Config, Env) {
    let conf = Config {
        max_trans_time: Duration::from_millis(200),
    };
    let env = Env::for_udp();
    (conf, env)
}

const DATA: &[u8] = b"When life gives you lemons, don't make lemonade. Make life take the lemons back! Get mad! I don't want your damn lemons, what the hell am I supposed to do with these? Demand to see life's manager! Make life rue the day it thought it could give Cave Johnson lemons! Do you know who I am? I'm the man who's gonna burn your house down! With the lemons! I'm gonna get my engineers to invent a combustible lemon that burns your house down!";

#[tokio::test]
async fn recv_one() {
    let (cl_tx, mut cl_rx) = tokio::sync::mpsc::channel::<Vec<u8>>(10);
    let (sv_tx, mut sv_rx) = tokio::sync::mpsc::channel::<Vec<u8>>(10);
    let server = spawn(async move {
        let (conf, env) = test_conf_env();
        let mut mech = ConnState::new(conf, env.clone());
        let mut scratch = vec![0u8; env.max_packet_size];
        let mut data = vec![];
        while let Some(packet) = sv_rx.recv().await {
            let read = packet::Read::try_read(&packet).unwrap();
            println!("server: processing");
            let res = mech.process(read, None, &mut scratch).await.unwrap();
            if let Some(write) = res.written {
                let buf = write.portion_to_send();
                cl_tx.send(buf.to_vec()).await.unwrap();
            }
            if let Some(read) = res.read {
                data.extend_from_slice(read);
            }
            if res.read_complete {
                return data;
            }
        }
        panic!("read did not complete before disconnect")
    });
    let client = spawn(async move {
        let (_conf, env) = test_conf_env();
        let chunk_size = env.max_packet_size / 2;
        let mut send = Send::new(DATA);
        let mut scratch = vec![0u8; env.max_packet_size];
        let mut gen = packet::uid::Seq::new();

        let id = gen.next();
        let mut last;
        sv_tx
            .send(
                packet::Write::new(&mut scratch)
                    .unwrap()
                    .with_packet(id)
                    .with_responding_to(Uid::null())
                    .write_cmd(packet::CmdKind::Tx)
                    .unwrap()
                    .portion_to_send()
                    .to_vec(),
            )
            .await
            .unwrap();
        {
            let recv = cl_rx.recv().await.unwrap();
            let read = packet::Read::try_read(&recv).unwrap();
            let head = read.head();
            last = head.packet;
            assert_eq!(head.responding_to, id);
            assert_eq!(head.packet_ty, packet::Type::Command as _);
            match read {
                packet::Read::Cmd(cmd) => {
                    assert_eq!(cmd.kind(), Ok(packet::CmdKind::Confirm))
                }
                packet::Read::Frame(..) => panic!("expected cmd, got frame"),
            }
        }
        while let Some(chunk) = send.advance(chunk_size) {
            let id = gen.next();
            sv_tx
                .send(
                    packet::Write::new(&mut scratch)
                        .unwrap()
                        .with_packet(id)
                        .with_responding_to(last)
                        .write_frame_with(chunk)
                        .unwrap()
                        .portion_to_send()
                        .to_vec(),
                )
                .await
                .unwrap();
            let recv = cl_rx.recv().await.unwrap();
            let read = packet::Read::try_read(&recv).unwrap();
            let head = read.head();
            last = head.packet;
            assert_eq!(head.responding_to, id);
            assert_eq!(head.packet_ty, packet::Type::Command as _);
            match read {
                packet::Read::Cmd(cmd) => {
                    assert_eq!(cmd.kind(), Ok(packet::CmdKind::Confirm))
                }
                packet::Read::Frame(..) => panic!("expected cmd, got frame"),
            }
        }
        let id = gen.next();
        sv_tx
            .send(
                packet::Write::new(&mut scratch)
                    .unwrap()
                    .with_packet(id)
                    .with_responding_to(last)
                    .write_cmd(packet::CmdKind::Complete)
                    .unwrap()
                    .portion_to_send()
                    .to_vec(),
            )
            .await
            .unwrap();
        let recv = cl_rx.recv().await.unwrap();
        let read = packet::Read::try_read(&recv).unwrap();
        let head = read.head();
        assert_eq!(head.responding_to, id);
        assert_eq!(head.packet_ty, packet::Type::Command as _);
        match read {
            packet::Read::Cmd(cmd) => {
                assert_eq!(cmd.kind(), Ok(packet::CmdKind::Confirm))
            }
            packet::Read::Frame(..) => panic!("expected cmd, got frame"),
        }
    });
    let recvd = select! {
        _ = sleep(Duration::from_millis(700)) => panic!("timed out"),
        res = client => { res.unwrap(); server.await.unwrap() }
    };
    assert_eq!(recvd, DATA.to_vec());
}

#[tokio::test]
async fn send_one() {
    let (cl_tx, mut cl_rx) = tokio::sync::mpsc::channel::<Vec<u8>>(10);
    let (sv_tx, mut sv_rx) = tokio::sync::mpsc::channel::<Vec<u8>>(10);
    let server = spawn(async move {
        let (conf, env) = test_conf_env();
        let mut mech = ConnState::new(conf, env.clone());
        let mut scratch = vec![0u8; env.max_packet_size];
        let mut send = Send::new(DATA);
        while let Some(packet) = sv_rx.recv().await {
            let read = packet::Read::try_read(&packet).unwrap();
            println!("server: processing");
            let res = mech
                .process(read, Some(&mut send), &mut scratch)
                .await
                .unwrap();
            if let Some(write) = res.written {
                let buf = write.portion_to_send();
                cl_tx.send(buf.to_vec()).await.unwrap();
            }
            if let Some(_read) = res.read {
                panic!("did not expect to read data");
            }
            if res.read_complete {
                panic!("did not expect to read data");
            }
        }
        assert!(
            send.done_sending(),
            "write did not complete before disconnect"
        )
    });
    let client = spawn(async move {
        let (_conf, env) = test_conf_env();
        let mut scratch = vec![0u8; env.max_packet_size];
        let mut gen = packet::uid::Seq::new();
        let mut data: Vec<u8> = vec![];

        let id = gen.next();
        let mut last;
        sv_tx
            .send(
                packet::Write::new(&mut scratch)
                    .unwrap()
                    .with_packet(id)
                    .with_responding_to(Uid::null())
                    .write_cmd(packet::CmdKind::Rx)
                    .unwrap()
                    .portion_to_send()
                    .to_vec(),
            )
            .await
            .unwrap();
        {
            let recv = cl_rx.recv().await.unwrap();
            let read = packet::Read::try_read(&recv).unwrap();
            let head = read.head();
            last = head.packet;
            assert_eq!(head.responding_to, id);
            assert_eq!(head.packet_ty, packet::Type::Frame as _);
            match read {
                packet::Read::Cmd(..) => panic!("expected frame, got cmd"),
                packet::Read::Frame(frame) => {
                    data.extend_from_slice(frame.data().unwrap());
                }
            }
        }
        loop {
            let id = gen.next();
            sv_tx
                .send(
                    packet::Write::new(&mut scratch)
                        .unwrap()
                        .with_packet(id)
                        .with_responding_to(last)
                        .write_cmd(packet::CmdKind::Confirm)
                        .unwrap()
                        .portion_to_send()
                        .to_vec(),
                )
                .await
                .unwrap();
            let recv = cl_rx.recv().await.unwrap();
            let read = packet::Read::try_read(&recv).unwrap();
            let head = read.head();
            last = head.packet;
            assert_eq!(head.responding_to, id);
            assert_eq!(head.packet_ty, packet::Type::Command as _);
            match read {
                packet::Read::Cmd(cmd) => {
                    assert_eq!(cmd.kind(), Ok(packet::CmdKind::Complete));
                    break;
                }
                packet::Read::Frame(frame) => {
                    data.extend_from_slice(frame.data().unwrap());
                }
            }
        }
        data
    });
    let recvd = select! {
        _ = sleep(Duration::from_millis(700)) => panic!("timed out"),
        res = client => { let recvd = res.unwrap(); server.await.unwrap(); recvd }
    };
    assert_eq!(recvd, DATA.to_vec());
}
