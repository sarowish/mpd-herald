use std::fs::File;

use crate::{cache, config::CONFIG, mpd::SongInfo};
use anyhow::Result;
use bytes::BytesMut;
use image::{GenericImageView, codecs::jpeg::JpegEncoder, imageops::FilterType};
use notify_rust::{Hint, Image, Notification, NotificationHandle};

pub fn init(song: &SongInfo) -> Result<Notification> {
    let mut n = song
        .to_notification()
        .timeout(CONFIG.notification.timeout)
        .finalize();

    if let Some(art) = &song.album_art {
        n.hint(image_to_hint(art)?);
    }

    Ok(n)
}

pub fn update(handle: &mut NotificationHandle, song: &SongInfo) -> Result<NotificationHandle> {
    Ok(init(song)?.id(handle.id()).show()?)
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
