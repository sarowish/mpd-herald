use crate::{config::CONFIG, service::Service};
use anyhow::Result;

mod cache;
mod cli;
mod config;
mod mpd;
mod notification;
mod rpc;
mod scrobbling;
mod service;
mod utils;

#[tokio::main(flavor = "multi_thread")]
async fn main() -> Result<()> {
    tracing_subscriber::fmt::init();

    let matches = cli::get_matches();

    if matches.subcommand_matches("authenticate").is_some() {
        return scrobbling::Scrobbling::authenticate().await;
    }

    anyhow::ensure!(
        CONFIG.notification.enable
            || CONFIG.discord_rpc.enable
            || CONFIG.scrobbling.lastfm.as_ref().is_some_and(|c| c.enable),
        "all modules are disabled"
    );

    let mut service = Service::new().await?;

    service.run().await
}
