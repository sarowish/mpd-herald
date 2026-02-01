mod cache;
mod config;
mod mpd;
mod notification;
mod rpc;
mod utils;

#[tokio::main(flavor = "current_thread")]
async fn main() {
    mpd::connect_to_mpd().await.unwrap();
}
