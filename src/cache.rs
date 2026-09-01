use crate::utils;
use anyhow::Result;
use bytes::BytesMut;
use std::{ffi::OsStr, path::PathBuf, time::SystemTime};
use tracing::error;

fn hash_image(bytes: &BytesMut) -> String {
    let mut s = blake3::Hasher::new();
    s.update(bytes);
    s.update(b"v1");

    let mut out = [0u8; 12];
    s.finalize_xof().fill(&mut out);

    hex::encode(out)
}

pub fn get_cached_image_path(bytes: &BytesMut) -> Result<PathBuf> {
    let hash = hash_image(bytes);

    let mut path = utils::get_cache_dir()?.join(hash);
    path.set_extension("jpg");

    Ok(path)
}

struct CacheEntry {
    path: PathBuf,
    size: u64,
    timestamp: SystemTime,
}

fn read_cache() -> Result<(u64, Vec<CacheEntry>)> {
    let path = utils::get_cache_dir()?;
    let mut entries = Vec::new();
    let mut total_size: u64 = 0;

    for entry in path.read_dir()?.flatten() {
        let path = entry.path();

        if path.extension() != Some(OsStr::new("jpg")) {
            continue;
        }

        let Ok(metadata) = entry.metadata() else {
            continue;
        };

        if !metadata.is_file() {
            continue;
        }

        let size = metadata.len();
        total_size = total_size.saturating_add(size);

        entries.push(CacheEntry {
            path,
            size,
            timestamp: metadata
                .accessed()
                .or_else(|_| metadata.modified())
                .or_else(|_| metadata.created())
                .unwrap_or_else(|_| SystemTime::now()),
        });
    }

    Ok((total_size, entries))
}

pub fn prune_images() -> Result<usize> {
    const MAX_CACHE_SIZE: u64 = 16 * 1024 * 1024;
    const TARGET_CACHE_SIZE: u64 = 12 * 1024 * 1024;

    let (mut total_size, mut entries) = read_cache()?;

    if total_size <= MAX_CACHE_SIZE {
        return Ok(0);
    }

    let mut count = 0;
    entries.sort_unstable_by_key(|a| a.timestamp);

    for entry in entries {
        if let Err(e) = std::fs::remove_file(&entry.path) {
            error!(
                "Couldn't remove {} while pruning cache: {e}",
                entry.path.display()
            );
            continue;
        }

        total_size = total_size.saturating_sub(entry.size);
        count += 1;

        if total_size <= TARGET_CACHE_SIZE {
            break;
        }
    }

    Ok(count)
}
