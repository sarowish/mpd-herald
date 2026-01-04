mod cache;
mod config;
mod mpd;
mod notification;
mod rpc;
mod utils;

use discord_presence::Client as DiscordClient;
use std::time::Duration;

const ID: u64 = 1465967948861669469;

#[tokio::main(flavor = "current_thread")]
async fn main() {
    let mut drpc = DiscordClient::with_error_config(ID, Duration::from_secs(3), Some(0));

    drpc.start();
    mpd::connect_to_mpd(drpc).await.unwrap();
}
