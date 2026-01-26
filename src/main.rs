mod cache;
mod config;
mod mpd;
mod notification;
mod rpc;
mod utils;

use crate::{config::CONFIG, rpc::RpcEvent};
use discord_presence::Client as DiscordClient;
use std::time::Duration;
use tokio::sync::mpsc;

#[tokio::main(flavor = "current_thread")]
async fn main() {
    let drpc = DiscordClient::with_error_config(
        CONFIG.discord_rpc.client_id,
        Duration::from_secs(3),
        None,
    );

    let (tx, rx) = mpsc::channel(16);

    let tx2 = tx.clone();

    drpc.on_connected(move |_| {
        tx.try_send(RpcEvent::Ready).unwrap();
    })
    .persist();
    drpc.on_disconnected(move |_| tx2.try_send(RpcEvent::NotConnected).unwrap())
        .persist();

    mpd::connect_to_mpd(drpc, rx).await.unwrap();
}
