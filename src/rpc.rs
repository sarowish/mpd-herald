use crate::mpd::SongInfo;
use anyhow::Result;
use discord_presence::{
    Client as DiscordClient,
    models::{ActivityTimestamps, ActivityType, DisplayType},
};
use mpd_client::{responses::PlayState, tag::Tag};
use reqwest::Client;
use std::{
    collections::HashMap,
    fmt::Display,
    sync::LazyLock,
    time::{SystemTime, UNIX_EPOCH},
};
use tokio::sync::Mutex;

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
                ReleaseType::Release => String::from("release"),
                ReleaseType::ReleaseGroup => String::from("release-group"),
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
        .or(song
            .single_tag_value(&release_group_id_tag)
            .map(|id| (ReleaseType::ReleaseGroup, id.to_owned())))
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
    } else if let ReleaseType::ReleaseGroup = rel_type {
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
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("Couldn't get system time")
        .as_secs();

    let timestamps = ActivityTimestamps::new();

    let Some(elapsed) = song.elapsed else {
        return timestamps;
    };

    let Some(duration) = song.duration else {
        return timestamps;
    };

    let start = now - elapsed.as_secs();
    let end = start + duration.as_secs();

    timestamps.start(start).end(end)
}

pub async fn update(drpc: &mut DiscordClient, song: SongInfo) {
    if song.state != PlayState::Playing {
        let _ = drpc.clear_activity();
        return;
    }

    let image_url = get_albumart(&song).await;

    let _ = drpc.set_activity(|act| {
        act.state(song.single_tag_value(&Tag::Artist).unwrap_or_default())
            .activity_type(ActivityType::Listening)
            .details(song.single_tag_value(&Tag::Title).unwrap_or_default())
            .status_display(DisplayType::State)
            .assets(|mut assets| {
                assets = assets.large_text(song.single_tag_value(&Tag::Album).unwrap_or_default());

                if let Ok(url) = image_url {
                    assets = assets.large_image(url);
                }

                assets
            })
            .timestamps(|_| build_timestamp(&song))
    });
}
