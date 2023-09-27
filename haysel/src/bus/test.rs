use std::{
    collections::HashMap,
    sync::{
        atomic::{self, AtomicBool},
        Arc,
    },
    time::Duration,
};

use dabus::extras::DynVar;
use tracing_test::traced_test;
use uuid::Uuid;

use super::{
    handler::{
        async_fn_ptr::HandlerFn, bus_dispatch_event, handler_task_rt_launch, Interface, Method,
    },
    id::{const_uuid_v4, Uid},
    msg::{self, Str},
    Bus,
};

#[tokio::test]
#[traced_test]
async fn bus_send_message() {
    let bus = Bus::new().await;
    const HDL_ID: Uuid = const_uuid_v4();
    struct Handler;
    impl Handler {
        async fn function_1(&mut self, args: &Arc<AtomicBool>, _interface: Interface) -> () {
            args.store(true, atomic::Ordering::Relaxed);
        }
    }
    const HDL_FN_1_ID: Uuid = const_uuid_v4();
    let instance_id = handler_task_rt_launch(
        bus.uid_src.clone(),
        bus.comm.clone(),
        bus.mgmnt_comm.clone(),
        HDL_ID,
        DynVar::new(Handler),
        Str::from("Test handler"),
        HashMap::from([(
            HDL_FN_1_ID,
            Method {
                handler_func: Box::new(HandlerFn::new(Handler::function_1)),
                handler_desc: Str::from("Test handler function 1"),
            },
        )]),
    )
    .await;
    let flag = Arc::new(AtomicBool::new(false));
    bus_dispatch_event(
        bus.uid_src.clone(),
        bus.comm.clone(),
        bus.mgmnt_comm.clone(),
        msg::HandlerInstance {
            typ: msg::HandlerType {
                id: const_uuid_v4(),
                id_desc: Str::from("fake sending handler"),
            },
            discriminant: Uid::gen_with(&bus.uid_src),
            discriminant_desc: Str::from("fake sending handler 1"),
        },
        msg::Target::Instance(instance_id),
        msg::MethodID {
            id: HDL_FN_1_ID,
            id_desc: Str::from("Test handler function 1"),
        },
        DynVar::new(flag.clone()),
    )
    .await
    .unwrap();
    tokio::time::sleep(Duration::from_secs(1)).await;
    let value = flag.load(atomic::Ordering::Relaxed);
    assert!(value, "handler did not run");
}
