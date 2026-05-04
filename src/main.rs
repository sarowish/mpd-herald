use crate::{config::CONFIG, service::Service};
use anyhow::Result;

mod cache;
mod config;
mod mpd;
mod notification;
mod rpc;
mod service;
mod utils;

#[tokio::main(flavor = "multi_thread")]
async fn main() -> Result<()> {
    tracing_subscriber::fmt::init();

    anyhow::ensure!(
        CONFIG.notification.enable || CONFIG.discord_rpc.enable,
        "both notification and discord_rpc are disabled"
    );

    let mut service = Service::new().await?;

    service.run().await
}
