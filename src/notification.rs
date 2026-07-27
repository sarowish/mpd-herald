use crate::{
    cache,
    config::CONFIG,
    mpd::{SongInfo, get_image},
};
use anyhow::Result;
use async_channel::{Receiver, Sender};
use bytes::BytesMut;
use image::{GenericImageView, codecs::jpeg::JpegEncoder, imageops::FilterType};
use mpd_client::{Client as MpdClient, responses::PlayState};
use notify_rust::{Hint, Image, Notification, NotificationHandle};
use std::fs::File;
use tracing::{warn, info};

struct NotificationText {
    summary: String,
    body: String,
}

impl From<&SongInfo> for NotificationText {
    fn from(song: &SongInfo) -> Self {
        let notification_config = &CONFIG.notification;

        let (summary, body) = match song.state {
            PlayState::Stopped => (
                notification_config.stopped_text.summary.render(song),
                notification_config.stopped_text.body.render(song),
            ),
            PlayState::Playing => (
                notification_config.playing_text.summary.render(song),
                notification_config.playing_text.body.render(song),
            ),
            PlayState::Paused => (
                notification_config.paused_text.summary.render(song),
                notification_config.paused_text.body.render(song),
            ),
        };

        Self { summary, body }
    }
}

pub struct NotificationEvent {
    pub client: MpdClient,
    pub song: SongInfo,
}

pub type NotificationSender = Sender<NotificationEvent>;
type NotificationReceiver = Receiver<NotificationEvent>;

pub fn spawn() -> Option<NotificationSender> {
    if !CONFIG.notification.enable {
        return None;
    }

    let (notification_tx, notification_rx) = async_channel::bounded(1);

    tokio::spawn(async move { run(notification_rx).await });

    Some(notification_tx)
}

pub async fn run(rx: NotificationReceiver) -> Result<()> {
    match cache::prune_images() {
        Ok(0) => (),
        Ok(count) => info!("Pruned {count} cached images"),
        Err(e) => warn!("Failed to prune image cache: {e}"),
    }

    let mut handle = None;

    while let Ok(event) = rx.recv().await {
        let art = get_image(&event.client, &event.song.url)
            .await
            .ok()
            .flatten();

        if let Some(h) = &mut handle {
            update(h, &event.song, art).await?;
        } else {
            handle = Some(init(&event.song, art).await?.show()?);
        }
    }

    Ok(())
}

pub async fn init(song: &SongInfo, art: Option<BytesMut>) -> Result<Notification> {
    let text = NotificationText::from(song);

    let mut n = Notification::new()
        .summary(&text.summary)
        .body(&text.body)
        .timeout(CONFIG.notification.timeout)
        .finalize();

    if !song.url.is_empty()
        && let Some(art) = art
    {
        n.hint(image_to_hint(&art)?);
    }

    Ok(n)
}

pub async fn update(
    handle: &mut NotificationHandle,
    song: &SongInfo,
    art: Option<BytesMut>,
) -> Result<()> {
    **handle = init(song, art).await?;
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
