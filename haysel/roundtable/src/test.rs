use std::{
    convert::Infallible,
    sync::{
        atomic::{self, AtomicBool},
        Arc,
    },
    time::Duration,
};

use tracing_test::traced_test;

use super::{
    common::HDL_EXTERNAL,
    handler::{HandlerInit, LocalInterface, MethodRegister},
    handler_decl_t, method_decl,
    msg::{HandlerType, Str},
    Bus,
};

#[traced_test]
#[test]
fn bus_send_message_rt() {
    tokio::runtime::Builder::new_multi_thread()
        .enable_time()
        .build()
        .unwrap()
        .block_on(bus_send_message());
}

async fn bus_send_message() {
    let bus = Bus::new().await;
    method_decl!(METHOD_1, Arc<AtomicBool>, ());
    struct Handler;
    impl Handler {
        async fn function_1(
            &mut self,
            args: &Arc<AtomicBool>,
            _: &LocalInterface,
        ) -> Result<(), <Self as HandlerInit>::Error> {
            args.store(true, atomic::Ordering::Relaxed);
            Ok(())
        }
    }
    impl HandlerInit for Handler {
        const DECL: HandlerType = handler_decl_t!("Test handler");
        type Error = Infallible;
        fn describe(&self) -> Str {
            Str::Borrowed("Test handler instance")
        }
        fn methods(&self, register: &mut MethodRegister<Self>) {
            register.register(Self::function_1, METHOD_1)
        }
    }
    let instance_id = bus.interface().spawn(Handler);

    let flag = Arc::new(AtomicBool::new(false));
    bus.interface()
        .query_as(HDL_EXTERNAL, instance_id, METHOD_1, flag.clone())
        .await
        .unwrap();
    tokio::time::sleep(Duration::from_secs(1)).await;
    let value = flag.load(atomic::Ordering::Relaxed);
    assert!(value, "handler did not run");
}
