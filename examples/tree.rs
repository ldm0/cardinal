extern crate cardinal;

use anyhow::{Context, Result};
use tokio::sync::oneshot;
use tracing::info;

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt().with_env_filter("debug").init();
    info!("Initializing cardinal sdk..");
    cardinal::init_sdk_facade();
    let (sender, receiver) = oneshot::channel();
    let mut sender = Some(sender);
    ctrlc::set_handler(move || {
        info!("Ctrl-C pressed");
        if let Some(sender) = sender.take() {
            info!("Closing cardinal sdk..");
            cardinal::close_sdk_facade();
            // ctrlc may be pressed multiple times.
            sender.send(()).unwrap();
        }
    })
    .context("Set handler failed")?;
    receiver.await.context("Exited with no ctrlc")?;
    Ok(())
}
