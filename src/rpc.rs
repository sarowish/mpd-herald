use crate::{
    config::CONFIG,
    mpd::{SongInfo, SongUpdate},
};
use anyhow::Result;
use discord_presence::{
    Client as DiscordClient,
    models::{Activity, ActivityTimestamps, ActivityType},
};
use mpd_client::{client::Subsystem, responses::PlayState, tag::Tag};
use reqwest::Client;
use std::{collections::HashMap, fmt::Display, sync::LazyLock};
use tokio::sync::{Mutex, watch};
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

enum ReleaseType {
    Release,
    ReleaseGroup,
}

impl Display for ReleaseType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{}",
            match self {
                Self::Release => String::from("release"),
                Self::ReleaseGroup => String::from("release-group"),
            }
        )
    }
}

#[derive(Default)]
struct AlbumArtClient {
    client: Client,
    cache: HashMap<String, String>,
}

async fn get_albumart(song: &SongInfo) -> Result<String> {
    static CLIENT: LazyLock<Mutex<AlbumArtClient>> =
        LazyLock::new(|| Mutex::new(AlbumArtClient::default()));

    let mut guard = CLIENT.lock().await;

    let release_group_id_tag = Tag::Other("MUSICBRAINZ_RELEASEGROUPID".into());
    let Some((rel_type, id)) = song
        .single_tag_value(&Tag::MusicBrainzReleaseId)
        .map(|id| (ReleaseType::Release, id.to_owned()))
        .or_else(|| {
            song.single_tag_value(&release_group_id_tag)
                .map(|id| (ReleaseType::ReleaseGroup, id.to_owned()))
        })
    else {
        return Err(anyhow::anyhow!("No musicbrainz id"));
    };

    if let Some(url) = guard.cache.get(&id) {
        return Ok(url.to_owned());
    }

    let url = format!("https://coverartarchive.org/{rel_type}/{id}/front-250");
    let Ok(resp) = guard.client.get(&url).send().await else {
        return Ok(url);
    };

    if let 200 | 307 = resp.status().as_u16() {
        let url = resp.url().to_string();
        guard.cache.insert(id, url.clone());
        return Ok(url);
    } else if matches!(rel_type, ReleaseType::ReleaseGroup) {
        return Err(anyhow::anyhow!("No image"));
    }

    let Some(url) = song
        .single_tag_value(&release_group_id_tag)
        .map(|id| format!("https://coverartarchive.org/release-group/{id}/front-250"))
    else {
        return Err(anyhow::anyhow!("No musicbrainz release group id"));
    };

    Ok(match guard.client.get(&url).send().await {
        Ok(resp) if matches!(resp.status().as_u16(), 200 | 307) => resp.url().to_string(),
        _ => url,
    })
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

    let image_url = get_albumart(song).await;

    let activity = |act: Activity| {
        act.activity_type(ActivityType::Listening)
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
            .timestamps(|_| build_timestamp(song))
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
