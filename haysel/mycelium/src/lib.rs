//! data sent over IPC should be serialized with json

#[macro_use]
extern crate serde;
#[macro_use]
extern crate thiserror;
extern crate tracing;

use std::{collections::HashMap, iter::repeat};

use chrono::{DateTime, Utc};
use serde::{de::DeserializeOwned, Serialize};
pub use squirrel;
pub use squirrel::api::station;
use squirrel::api::station::{
    capabilities::{Channel, ChannelData, ChannelID, KnownChannels},
    identity::{KnownStations, StationID},
};
use tokio::io::{self, AsyncReadExt, AsyncWriteExt};

#[derive(Debug, Error)]
pub enum IPCError {
    #[error("IO Operation failed {0}")]
    IO(#[from] io::Error),
    #[error("Deserialize failed {0:?}")]
    Deserialize(#[from] rmp_serde::decode::Error),
    #[error("Serialization failed {0:?}")]
    Serialize(#[from] rmp_serde::encode::Error),
    #[error("Reader reached EOF")]
    EOF,
}

/// Write a IPC packet to a stream.
///
/// Receive packet with `ipc_recv`
pub async fn ipc_send<T: Serialize>(
    socket: &mut (impl AsyncWriteExt + Unpin),
    packet: &T,
) -> Result<(), IPCError> {
    let serialized = rmp_serde::to_vec_named(packet)?;
    let len_bytes = (serialized.len() as u64).to_be_bytes();
    socket.write_all(&len_bytes).await?;
    socket.write_all(&serialized).await?;
    Ok(())
}

/// Reads an IPC packet from `socket`
///
/// this will only work if *every previous packet received was correct*
/// or if the stream was 'reset', as in no bytes from previous packets are left over
pub async fn ipc_recv<T: DeserializeOwned>(
    socket: &mut (impl AsyncReadExt + Unpin),
) -> Result<T, IPCError> {
    let mut buf = [0u8; 8]; //u64
    socket.read_exact(&mut buf).await?;
    let amnt = u64::from_be_bytes(buf);
    let mut buf = vec![0u8; amnt as _];
    socket.read_exact(&mut buf).await?;
    Ok(rmp_serde::from_slice(&buf)?)
}

/// same as ipc_recv, but cancel safe
pub async fn ipc_recv_cancel_safe<T: DeserializeOwned>(
    buffer: &mut Vec<u8>,
    amnt: &mut usize,
    socket: &mut (impl AsyncReadExt + Unpin),
) -> Result<T, IPCError> {
    loop {
        assert!(*amnt <= buffer.len());
        if *amnt == buffer.len() {
            buffer.extend(repeat(0).take(128));
        }
        if *amnt < 8 {
            match socket.read(&mut buffer[*amnt..]).await? {
                0 => return Err(IPCError::EOF),
                n => *amnt += n,
            }
        } else {
            let the_rest = u64::from_be_bytes(buffer[..8].try_into().unwrap()) as usize;
            if *amnt < 8 + the_rest {
                match socket.read(&mut buffer[*amnt..]).await? {
                    0 => return Err(IPCError::EOF),
                    n => *amnt += n,
                }
            } else {
                buffer.copy_within(8 + the_rest..*amnt, 0);
                *amnt = 0;
                return Ok(rmp_serde::from_slice(&buffer[8..][..the_rest])?);
            }
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IPCMsg {
    pub kind: IPCMsgKind,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum IPCMsgKind {
    // -- server to client --
    /// Initialization packet, sent with current information about stuff
    Haiii {
        stations: KnownStations,
        channels: KnownChannels,
    },
    /// server disconnect ʘ︵ʘ
    Bye,
    FreshHotData {
        from: StationID,
        recorded_at: DateTime<Utc>,
        by_channel: HashMap<ChannelID, ChannelData>,
    },
    NewStation {
        id: StationID,
    },
    NewChannel {
        id: ChannelID,
        ch: Channel,
    },
    StationNewChannel {
        station: StationID,
        channel: ChannelID,
    },
    // response to QueryLastHourOf
    QueryLastHourResponse {
        data: Vec<(DateTime<Utc>, f32)>,
        from_time: DateTime<Utc>,
    },
    /// -- client to server --
    ClientDisconnect,
    QueryLastHourOf {
        station: StationID,
        channel: ChannelID,
    },
}
