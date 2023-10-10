use tokio::{select, signal::ctrl_c};

use super::ShutdownHandle;

pub async fn trap_ctrl_c(mut handle: ShutdownHandle) {
    warn!("Trapping ctrl+c, it will be useless until initialization is finished");
    tokio::spawn(async move {
        select! {
            res = ctrl_c() => {
                if let Err(_) = res {
                    error!("Failed to listen for ctrl_c signal - triggering shutdown");
                }
                info!("shutdown triggered");
                handle.trigger_shutdown();
            }
            _ = handle.wait_for_shutdown() => {}
        }
    });
}
