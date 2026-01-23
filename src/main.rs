mod cache;
mod config;
mod mpd;
mod notification;
mod rpc;
mod utils;

use discord_presence::Client as DiscordClient;
use std::time::Duration;

use crate::config::CONFIG;

#[tokio::main(flavor = "current_thread")]
async fn main() {
    let mut drpc = DiscordClient::with_error_config(
        CONFIG.discord_rpc.client_id,
        Duration::from_secs(3),
        Some(0),
    );

    drpc.start();
    mpd::connect_to_mpd(drpc).await.unwrap();
}
