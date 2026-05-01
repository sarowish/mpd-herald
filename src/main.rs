mod cache;
mod config;
mod mpd;
mod notification;
mod rpc;
mod utils;

#[tokio::main(flavor = "multi_thread")]
async fn main() {
    tracing_subscriber::fmt::init();
    mpd::connect_to_mpd().await.unwrap();
}
