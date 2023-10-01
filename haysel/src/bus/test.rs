use std::{
    sync::{
        atomic::{self, AtomicBool},
        Arc,
    },
    time::Duration,
};

use tracing_test::traced_test;

use super::{
    common::HDL_EXTERNAL,
    handler::{handler_decl_t, method_decl, HandlerInit, Interface, MethodRegister},
    msg::{self, HandlerType, Str},
    Bus,
};

#[tokio::test]
#[traced_test]
async fn bus_send_message() {
    let bus = Bus::new().await;
    method_decl!(METHOD_1, Arc<AtomicBool>, ());
    struct Handler;
    impl Handler {
        async fn function_1(&mut self, args: &Arc<AtomicBool>, _interface: Interface) -> () {
            args.store(true, atomic::Ordering::Relaxed);
        }
    }
    impl HandlerInit for Handler {
        const DECL: HandlerType = handler_decl_t!("Test handler");
        fn describe(&self) -> Str {
            Str::Borrowed("Test handler instance")
        }
        fn methods(&self, register: &mut MethodRegister<Self>) {
            register.register(Self::function_1, METHOD_1)
        }
    }
    let instance_id = bus.interface().spawn(Handler).await;

    let flag = Arc::new(AtomicBool::new(false));
    bus.interface()
        .dispatch_as(
            HDL_EXTERNAL,
            msg::Target::Instance(instance_id),
            METHOD_1,
            flag.clone(),
        )
        .await
        .unwrap()
        .unwrap();
    tokio::time::sleep(Duration::from_secs(1)).await;
    let value = flag.load(atomic::Ordering::Relaxed);
    assert!(value, "handler did not run");
}
