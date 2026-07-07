use anyhow::{Result, bail};
use std::{
    path::PathBuf,
    time::{Duration, SystemTime, UNIX_EPOCH},
};
use tokio::process::Command;

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

pub fn get_state_dir() -> Result<PathBuf> {
    let path = match dirs::state_dir() {
        Some(path) => path.join(PACKAGE_NAME),
        None => bail!("Couldn't find state directory"),
    };

    Ok(path)
}

pub fn get_config_dir() -> Result<PathBuf> {
    let path = match dirs::config_dir() {
        Some(path) => path.join(PACKAGE_NAME),
        None => bail!("Couldn't find config directory"),
    };

    Ok(path)
}

pub fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("Couldn't get system time")
        .as_secs()
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

pub async fn run(mut command: Command) -> Result<()> {
    let mut child = command.spawn()?;

    let exit_status = child.wait().await?;

    if let Some(code) = exit_status.code()
        && code != 0
    {
        Err(anyhow::anyhow!("Process exited with status code {code}"))
    } else {
        Ok(())
    }
}

pub async fn open_in_browser(url: &str) -> Result<()> {
    let commands = open::commands(url);
    let mut last_error = None;

    for cmd in commands {
        let command = Command::from(cmd);

        match run(command).await {
            Ok(()) => return Ok(()),
            Err(err) => last_error = Some(err),
        }
    }

    match last_error {
        Some(error) => Err(error),
        None => Ok(()),
    }
}
