use crate::utils;
use anyhow::Result;
use bytes::BytesMut;
use std::path::PathBuf;

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
