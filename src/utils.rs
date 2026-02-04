use anyhow::{Result, bail};
use std::{path::PathBuf, time::Duration};

const PACKAGE_NAME: &str = env!("CARGO_PKG_NAME");

pub fn get_cache_dir() -> Result<PathBuf> {
    let path = match dirs::cache_dir() {
        Some(path) => path.join(PACKAGE_NAME),
        None => bail!("Couldn't find cache directory"),
    };

    if !path.exists() {
        std::fs::create_dir_all(&path)?;
    }

    Ok(path)
}

pub fn get_config_dir() -> Result<PathBuf> {
    let path = match dirs::config_dir() {
        Some(path) => path.join(PACKAGE_NAME),
        None => bail!("Couldn't find config directory"),
    };

    Ok(path)
}

pub fn duration_as_hhmmss(duration: Option<Duration>) -> String {
    let Some(duration) = duration.map(|d| d.as_secs()) else {
        return String::from("-");
    };

    let seconds = duration % 60;
    let minutes = (duration / 60) % 60;
    let hours = (duration / 60) / 60;
    match (hours, minutes, seconds) {
        (0, 0, _) => format!("0:{seconds:02}"),
        (0, _, _) => format!("{minutes}:{seconds:02}"),
        _ => format!("{hours}:{minutes:02}:{seconds:02}"),
    }
}
