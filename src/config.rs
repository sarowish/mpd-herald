use std::{fs, sync::LazyLock};

use crate::{mpd::SongInfo, utils};
use anyhow::Result;
use regex_lite::Regex;
use serde::Deserialize;

pub static CONFIG: LazyLock<Config> = LazyLock::new(|| match Config::new() {
    Ok(config) => config,
    Err(e) => {
        eprintln!("{e:?}");
        std::process::exit(1);
    }
});

const CONFIG_FILE: &str = "config.toml";

fn default_host() -> String {
    String::from("localhost")
}

fn default_port() -> u32 {
    6600
}

fn default_timeout() -> i32 {
    6000
}

fn default_playing_text() -> NotificationText {
    NotificationText {
        summary: String::from("  %title%"),
        body: String::from("%albumartist% - %album%"),
    }
}

fn default_paused_text() -> NotificationText {
    NotificationText {
        summary: String::from("  %title%"),
        body: String::from("%albumartist% - %album%"),
    }
}

fn default_stopped_text() -> NotificationText {
    NotificationText {
        summary: String::from("Stopped"),
        body: String::default(),
    }
}

#[derive(Deserialize)]
pub struct NotificationText {
    pub summary: String,
    pub body: String,
}

fn extract_tokens(format: &str) -> Vec<String> {
    static RE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"%\w+%").unwrap());

    RE.captures_iter(format)
        .map(|caps| caps[0].to_string())
        .collect::<Vec<_>>()
}

fn replace_tokens(format: &str, tokens: &Vec<String>, song: &SongInfo) -> String {
    let mut compiled_string = format.to_owned();

    for token in tokens {
        let value = song.get_token_value(token);
        compiled_string = compiled_string.replace(token, &value);
    }

    compiled_string
}

pub fn format_notification_text(format: &str, song: &SongInfo) -> String {
    replace_tokens(format, &extract_tokens(format), song)
}

#[derive(Deserialize)]
pub struct Config {
    #[serde(default = "default_host")]
    pub host: String,
    #[serde(default = "default_port")]
    pub port: u32,
    #[serde(default = "default_timeout")]
    pub timeout: i32,
    #[serde(default = "default_playing_text")]
    pub playing_text: NotificationText,
    #[serde(default = "default_paused_text")]
    pub paused_text: NotificationText,
    #[serde(default = "default_stopped_text")]
    pub stopped_text: NotificationText,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            host: default_host(),
            port: default_port(),
            timeout: default_timeout(),
            playing_text: default_playing_text(),
            paused_text: default_paused_text(),
            stopped_text: default_stopped_text(),
        }
    }
}

impl Config {
    pub fn new() -> Result<Self> {
        let config_path = utils::get_config_dir()?.join(CONFIG_FILE);
        let config_str = fs::read_to_string(config_path);

        Ok(if let Ok(config_str) = &config_str {
            toml::from_str::<Config>(config_str)?
        } else {
            Config::default()
        })
    }
}
