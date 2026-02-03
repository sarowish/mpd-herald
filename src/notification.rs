use std::fs::File;

use crate::{
    cache,
    config::{CONFIG, format_notification_text},
    mpd::{SongInfo, get_image},
};
use anyhow::Result;
use bytes::BytesMut;
use image::{GenericImageView, codecs::jpeg::JpegEncoder, imageops::FilterType};
use mpd_client::{Client, responses::PlayState};
use notify_rust::{Hint, Image, Notification, NotificationHandle};

struct NotificationText {
    summary: String,
    body: String,
}

impl From<&SongInfo> for NotificationText {
    fn from(value: &SongInfo) -> Self {
        let notification_config = &CONFIG.notification;

        let (summary, body) = match value.state {
            PlayState::Stopped => (
                format_notification_text(&notification_config.stopped_text.summary, value),
                format_notification_text(&notification_config.stopped_text.body, value),
            ),
            PlayState::Playing => (
                format_notification_text(&notification_config.playing_text.summary, value),
                format_notification_text(&notification_config.playing_text.body, value),
            ),
            PlayState::Paused => (
                format_notification_text(&notification_config.paused_text.summary, value),
                format_notification_text(&notification_config.paused_text.body, value),
            ),
        };

        Self { summary, body }
    }
}

pub async fn init(client: &Client, song: &SongInfo) -> Result<Notification> {
    let text = NotificationText::from(song);

    let mut n = Notification::new()
        .summary(&text.summary)
        .body(&text.body)
        .timeout(CONFIG.notification.timeout)
        .finalize();

    let album_art = get_image(client, &song.url).await?;

    if let Some(art) = album_art {
        n.hint(image_to_hint(&art)?);
    }

    Ok(n)
}

pub async fn update(
    handle: &mut NotificationHandle,
    client: &Client,
    song: &SongInfo,
) -> Result<()> {
    **handle = init(client, song).await?;
    handle.update();

    Ok(())
}

fn image_to_hint(bytes: &BytesMut) -> Result<Hint> {
    let cached = cache::get_cached_image_path(bytes);

    if let Ok(path) = &cached
        && path.exists()
    {
        return Ok(Hint::ImagePath(path.to_string_lossy().to_string()));
    }

    let mut image = image::load_from_memory(bytes)?;
    image = image.resize(128, 128, FilterType::Gaussian);

    if let Ok(path) = cached {
        let file = File::create(&path)?;
        let encoder = JpegEncoder::new_with_quality(file, 90);
        image.write_with_encoder(encoder)?;

        Ok(Hint::ImagePath(path.to_string_lossy().to_string()))
    } else {
        let (width, height) = image.dimensions();

        Ok(Hint::ImageData(Image::from_rgb(
            width.cast_signed(),
            height.cast_signed(),
            image.into_bytes(),
        )?))
    }
}
