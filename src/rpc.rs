use crate::{
    album_art,
    config::{CONFIG, DiscordRpcButton},
    mpd::{SongInfo, SongUpdate},
};
use discord_presence::{
    Client as DiscordClient,
    models::{Activity, ActivityTimestamps, ActivityType},
};
use mpd_client::{client::Subsystem, responses::PlayState};
use tokio::sync::watch;
use tracing::{error, info};

pub type RpcSender = async_channel::Sender<SongInfo>;
type RpcReceiver = async_channel::Receiver<SongInfo>;

pub fn spawn() -> Option<RpcSender> {
    if !CONFIG.discord_rpc.enable {
        return None;
    }

    let (rpc_tx, rpc_rx) = async_channel::bounded(1);

    tokio::spawn(async move { run(rpc_rx).await });

    Some(rpc_tx)
}

fn build_timestamp(song: &SongInfo) -> ActivityTimestamps {
    let timestamps = ActivityTimestamps::new();

    let Some(elapsed) = song.elapsed else {
        return timestamps;
    };

    let Some(duration) = song.duration else {
        return timestamps;
    };

    let start = song.fired_at - elapsed.as_secs();
    let end = start + duration.as_secs();

    timestamps.start(start).end(end)
}

fn add_buttons(activity: Activity, buttons: &[DiscordRpcButton], song: &SongInfo) -> Activity {
    buttons.iter().fold(activity, |activity, button| {
        let label = button.label.render(song);
        let url = button.url.render(song);

        activity.append_buttons(|activity_button| activity_button.label(label).url(url))
    })
}

async fn run(rx: RpcReceiver) {
    let mut drpc = DiscordClient::new(CONFIG.discord_rpc.client_id);

    let (connection_tx, mut connection_rx) = watch::channel(false);
    let connected_tx = connection_tx.clone();

    drpc.on_connected(move |_| {
        info!("[Discord Rpc] Connected");
        connected_tx.send_replace(true);
    })
    .persist();
    drpc.on_disconnected(move |_| {
        info!("[Discord Rpc] Disconnected");
        connection_tx.send_replace(false);
    })
    .persist();

    drpc.start();

    let mut latest_song = None;
    let mut connected = false;

    loop {
        tokio::select! {
            result = rx.recv() => {
                let Ok(song) = result else {
                    break;
                };

                if connected {
                    let queue_activity = latest_song
                            .as_ref()
                            .is_some_and(|latest| song.check_update(latest, Subsystem::Player) == SongUpdate::Seeked);

                    update(&mut drpc, &song, queue_activity).await;
                }

                latest_song = Some(song);
            }

            result = connection_rx.changed() => {
                if result.is_err() {
                    break;
                }

                connected = *connection_rx.borrow_and_update();

                if connected
                    && let Some(latest_playing) = &latest_song
                    && latest_playing.state == PlayState::Playing
                {
                    update(&mut drpc, latest_playing, false).await;
                }
            }
        }
    }
}

async fn update(drpc: &mut DiscordClient, song: &SongInfo, queue: bool) {
    if song.state != PlayState::Playing {
        match drpc.clear_activity() {
            Ok(_) => info!("[Discord Rpc] Cleared activity"),
            Err(e) => error!("[Discord Rpc] {e:?}"),
        }
        return;
    }

    let rpc_config = &CONFIG.discord_rpc;

    let state = rpc_config.state.render(song);
    let details = rpc_config.details.render(song);
    let large_text = rpc_config.large_text.render(song);
    let small_text = rpc_config.small_text.render(song);

    let image_url = album_art::get(song).await;

    let activity = |act: Activity| {
        let activity = act
            .activity_type(ActivityType::Listening)
            .state(state)
            .details(details)
            .status_display((&rpc_config.display_type).into())
            .assets(|mut assets| {
                if !large_text.is_empty() {
                    assets = assets.large_text(large_text);
                }

                if !small_text.is_empty() {
                    assets = assets.small_text(small_text);
                }

                if let Ok(url) = image_url {
                    assets = assets.large_image(url);
                } else if !rpc_config.large_image.is_empty() {
                    assets = assets.large_image(&rpc_config.large_image);
                }

                if !rpc_config.small_image.is_empty() {
                    assets = assets.small_image(&rpc_config.small_image);
                }

                assets
            })
            .timestamps(|_| build_timestamp(song));

        add_buttons(activity, &rpc_config.buttons, song)
    };

    if queue {
        drpc.queue_activity(activity);
        info!("[Discord Rpc] Queued activity")
    } else {
        match drpc.set_activity(activity) {
            Ok(_) => info!("[Discord Rpc] Set activity"),
            Err(e) => error!("[Discord Rpc] {e:?}"),
        }
    }
}
