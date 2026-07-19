use crate::{config::CONFIG, service::Service};
use anyhow::Result;

mod cache;
mod cli;
mod config;
mod format;
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

    tokio::select! {
        res = service.run() => res,
        res = wait_for_shutdown_signal() => {
            res?;
            service.shutdown().await
        }
    }
}

async fn wait_for_shutdown_signal() -> Result<()> {
    use tokio::signal::ctrl_c;

    #[cfg(unix)]
    {
        use tokio::signal::unix::{SignalKind, signal};

        let mut sigterm = signal(SignalKind::terminate())?;

        tokio::select! {
            result = ctrl_c() => result?,
            _ = sigterm.recv() => {}
        }
    }

    #[cfg(not(unix))]
    ctrl_c().await?;

    Ok(())
}
