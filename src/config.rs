use std::{fs, sync::LazyLock};

use crate::{mpd::SongInfo, utils};
use anyhow::Result;
use discord_presence::models::DisplayType as PresenceDisplayType;
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

#[derive(Deserialize)]
pub struct NotificationText {
    pub summary: String,
    pub body: String,
}

impl NotificationText {
    pub fn new(summary: &str, body: &str) -> Self {
        Self {
            summary: summary.to_owned(),
            body: body.to_owned(),
        }
    }
}

pub fn extract_tokens(format: &str) -> Vec<String> {
    static RE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"%\w+%").unwrap());

    RE.captures_iter(format)
        .map(|caps| caps[0].to_string())
        .collect::<Vec<_>>()
}

pub fn replace_tokens(format: &str, tokens: &Vec<String>, song: &SongInfo) -> String {
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
#[serde(default)]
pub struct NotificationConfig {
    pub timeout: i32,
    pub playing_text: NotificationText,
    pub paused_text: NotificationText,
    pub stopped_text: NotificationText,
}

impl Default for NotificationConfig {
    fn default() -> Self {
        Self {
            timeout: 6000,
            playing_text: NotificationText::new("  %title%", "%albumartist% - %album%"),
            paused_text: NotificationText::new("  %title%", "%albumartist% - %album%"),
            stopped_text: NotificationText::new("Stopped", ""),
        }
    }
}

#[derive(Deserialize, Clone, Debug)]
#[serde(rename_all = "snake_case")]
pub enum DisplayType {
    Name = 0,
    State = 1,
    Details = 2,
}

impl From<&DisplayType> for PresenceDisplayType {
    fn from(value: &DisplayType) -> Self {
        match value {
            DisplayType::Name => Self::Name,
            DisplayType::State => Self::State,
            DisplayType::Details => Self::Details,
        }
    }
}

#[derive(Deserialize)]
#[serde(default)]
pub struct DiscordRpcConfig {
    pub client_id: u64,
    pub state: String,
    pub details: String,
    pub large_text: String,
    pub small_text: String,
    pub large_image: String,
    pub small_image: String,
    pub display_type: DisplayType,
}

impl Default for DiscordRpcConfig {
    fn default() -> Self {
        Self {
            client_id: 1465967948861669469,
            state: String::from("%albumartist%"),
            details: String::from("%title%"),
            large_text: String::from("%album%"),
            small_text: String::default(),
            large_image: String::default(),
            small_image: String::default(),
            display_type: DisplayType::State,
        }
    }
}

#[derive(Deserialize)]
#[serde(default)]
pub struct Config {
    pub host: String,
    pub port: u32,
    pub notification: NotificationConfig,
    pub discord_rpc: DiscordRpcConfig,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            host: String::from("localhost"),
            port: 6600,
            notification: NotificationConfig::default(),
            discord_rpc: DiscordRpcConfig::default(),
        }
    }
}

impl Config {
    pub fn new() -> Result<Self> {
        let config_path = utils::get_config_dir()?.join(CONFIG_FILE);
        let config_str = fs::read_to_string(config_path);

        Ok(if let Ok(config_str) = &config_str {
            toml::from_str::<Self>(config_str)?
        } else {
            Self::default()
        })
    }
}
