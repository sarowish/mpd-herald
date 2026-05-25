use crate::{
    config::CONFIG,
    utils::{self, duration_as_hhmmss},
};
use anyhow::Result;
use bytes::BytesMut;
use mpd_client::{Client, client::ConnectionEvents, commands, responses::PlayState, tag::Tag};
use std::{collections::HashMap, fmt::Display, time::Duration};
use tokio::net::TcpStream;

pub async fn connect() -> Result<(Client, ConnectionEvents)> {
    let connection = TcpStream::connect(format!("{}:{}", CONFIG.host, CONFIG.port)).await?;

    Ok(Client::connect(connection).await?)
}

pub async fn get_image(client: &Client, uri: &str) -> Result<Option<BytesMut>> {
    let (from_file, resp) = if let Some(resp) = client.command(commands::AlbumArt::new(uri)).await?
    {
        (true, resp)
    } else if let Some(resp) = client.command(commands::AlbumArtEmbedded::new(uri)).await? {
        (false, resp)
    } else {
        return Ok(None);
    };

    let mut out = resp.data;
    let expected_size = resp.size;
    out.reserve(expected_size);

    while out.len() < expected_size {
        let resp = if from_file {
            client
                .command(commands::AlbumArt::new(uri).offset(out.len()))
                .await?
        } else {
            client
                .command(commands::AlbumArtEmbedded::new(uri).offset(out.len()))
                .await?
        };

        if let Some(resp) = resp {
            out.extend_from_slice(&resp.data);
        } else {
            return Ok(None);
        }
    }

    Ok(Some(out))
}

#[derive(Clone)]
pub struct SongInfo {
    pub url: String,
    pub state: PlayState,
    pub elapsed: Option<Duration>,
    pub duration: Option<Duration>,
    pub tags: HashMap<Tag, Vec<String>>,
    pub fired_at: u64,
}

impl SongInfo {
    pub async fn new(client: &Client) -> Result<Self> {
        let fired_at = utils::now();

        let Some(song) = client
            .command(commands::CurrentSong)
            .await?
            .map(|song| song.song)
        else {
            return Ok(Self {
                url: String::new(),
                state: PlayState::Stopped,
                elapsed: None,
                duration: None,
                tags: HashMap::default(),
                fired_at,
            });
        };

        let status = client.command(commands::Status).await?;

        Ok(Self {
            url: song.url,
            state: status.state,
            elapsed: status.elapsed,
            duration: status.duration,
            tags: song.tags,
            fired_at,
        })
    }

    fn tag_values(&self, tag: &Tag) -> &[String] {
        match self.tags.get(tag) {
            Some(v) => v.as_slice(),
            None => &[],
        }
    }

    pub fn single_tag_value(&self, tag: &Tag) -> Option<&str> {
        match self.tag_values(tag) {
            [] => None,
            [v, ..] => Some(v),
        }
    }

    pub fn get_token_value(&self, token: &str) -> String {
        let token = token.trim_matches('%');

        match token {
            "name" => self.single_tag_value(&Tag::Name),
            "artist" => self.single_tag_value(&Tag::Artist),
            "album" => self.single_tag_value(&Tag::Album),
            "albumartist" => self.single_tag_value(&Tag::AlbumArtist),
            "composer" => self.single_tag_value(&Tag::Composer),
            "date" => self.single_tag_value(&Tag::Date),
            "originaldate" => self.single_tag_value(&Tag::OriginalDate),
            "disc" => self.single_tag_value(&Tag::Disc),
            "genre" => self.single_tag_value(&Tag::Genre),
            "performer" => self.single_tag_value(&Tag::Performer),
            "title" => self.single_tag_value(&Tag::Title),
            "track" => self.single_tag_value(&Tag::Track),
            "time" => return duration_as_hhmmss(self.duration),
            "elapsed" => return duration_as_hhmmss(self.elapsed),
            "file" => return self.url.clone(),
            _ => Some(token),
        }
        .unwrap_or_default()
        .to_string()
    }

    pub fn check_update(&self, other: &Self) -> SongUpdate {
        if self.state == PlayState::Stopped {
            return SongUpdate::Stopped;
        }

        match (self.tags == other.tags, self.state == other.state) {
            (true, true) => SongUpdate::Seeked,
            (true, false) => SongUpdate::ToggledState,
            (false, _) => SongUpdate::Changed,
        }
    }
}

#[derive(PartialEq, Eq)]
pub enum SongUpdate {
    Seeked,
    ToggledState,
    Changed,
    Stopped,
}

impl Display for SongInfo {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let song = format!(
            "{} - {} ({}/{})",
            self.single_tag_value(&Tag::AlbumArtist).unwrap_or_default(),
            self.single_tag_value(&Tag::Title).unwrap_or_default(),
            duration_as_hhmmss(self.elapsed),
            duration_as_hhmmss(self.duration),
        );

        write!(
            f,
            "{}",
            match self.state {
                PlayState::Stopped => String::from("Stopped"),
                PlayState::Playing => format!("Playing: {song}"),
                PlayState::Paused => format!("Paused: {song}"),
            }
        )
    }
}
