use std::{fs, sync::LazyLock};

use crate::{
    format::CompiledFormat,
    utils::{self},
};
use anyhow::Result;
use discord_presence::models::DisplayType as PresenceDisplayType;
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
#[serde(deny_unknown_fields)]
pub struct NotificationText {
    pub summary: CompiledFormat,
    pub body: CompiledFormat,
}

impl NotificationText {
    pub fn new(summary: &str, body: &str) -> Self {
        Self {
            summary: CompiledFormat::new(summary),
            body: CompiledFormat::new(body),
        }
    }
}

#[derive(Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct NotificationConfig {
    pub enable: bool,
    pub timeout: i32,
    pub playing_text: NotificationText,
    pub paused_text: NotificationText,
    pub stopped_text: NotificationText,
}

impl Default for NotificationConfig {
    fn default() -> Self {
        Self {
            enable: true,
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
#[serde(default, deny_unknown_fields)]
pub struct DiscordRpcConfig {
    pub enable: bool,
    pub client_id: u64,
    pub state: CompiledFormat,
    pub details: CompiledFormat,
    pub large_text: CompiledFormat,
    pub small_text: CompiledFormat,
    pub large_image: String,
    pub small_image: String,
    pub display_type: DisplayType,
}

impl Default for DiscordRpcConfig {
    fn default() -> Self {
        Self {
            enable: true,
            client_id: 1465967948861669469,
            state: CompiledFormat::new("%albumartist%"),
            details: CompiledFormat::new("%title%"),
            large_text: CompiledFormat::new("%album%"),
            small_text: CompiledFormat::default(),
            large_image: String::default(),
            small_image: String::default(),
            display_type: DisplayType::State,
        }
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LastFmConfig {
    pub enable: bool,
    pub api_key: String,
    pub secret: String,
    #[serde(default)]
    pub prefer_album_artist: bool,
}

#[derive(Deserialize, Default)]
#[serde(default, deny_unknown_fields)]
pub struct ScrobblingConfig {
    pub lastfm: Option<LastFmConfig>,
}

#[derive(Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Config {
    pub host: String,
    pub port: u32,
    pub notification: NotificationConfig,
    pub discord_rpc: DiscordRpcConfig,
    pub scrobbling: ScrobblingConfig,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            host: String::from("localhost"),
            port: 6600,
            notification: NotificationConfig::default(),
            discord_rpc: DiscordRpcConfig::default(),
            scrobbling: ScrobblingConfig::default(),
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
