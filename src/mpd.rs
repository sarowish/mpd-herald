use std::collections::HashMap;

use anyhow::Result;
use bytes::BytesMut;
use mpd_client::{
    Client,
    client::{ConnectionEvent, Subsystem},
    commands,
    responses::PlayState,
    tag::Tag,
};
use notify_rust::Notification;
use tokio::net::TcpStream;

use crate::{
    config::{CONFIG, format_notification_text},
    notification,
};

pub async fn connect_to_mpd() -> Result<()> {
    let connection = TcpStream::connect(format!("{}:{}", CONFIG.host, CONFIG.port)).await?;

    let (client, mut state_changes) = Client::connect(connection).await?;

    let mut handle = notification::init(SongInfo::new(&client).await?)?.show()?;

    loop {
        match state_changes.next().await {
            Some(ConnectionEvent::SubsystemChange(Subsystem::Player)) => {
                handle = notification::update(&mut handle, SongInfo::new(&client).await?)?;
            }
            Some(ConnectionEvent::SubsystemChange(_)) => (),
            _ => break,
        }
    }

    Ok(())
}

async fn get_image(client: &Client, uri: &str) -> Result<Option<BytesMut>> {
    let mut out = BytesMut::new();
    let mut expected_size = 0;
    let mut from_file = false;

    if let Some(resp) = client.command(commands::AlbumArt::new(uri)).await? {
        out = resp.data;
        expected_size = resp.size;
        out.reserve(expected_size);
        from_file = true;
    }

    if !from_file {
        if let Some(resp) = client.command(commands::AlbumArt::new(uri)).await? {
            out = resp.data;
            expected_size = resp.size;
            out.reserve(expected_size);
        } else {
            return Ok(None);
        }
    }

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

pub struct SongInfo {
    pub state: PlayState,
    pub tags: HashMap<Tag, Vec<String>>,
    pub album_art: Option<BytesMut>,
}

impl SongInfo {
    async fn new(client: &Client) -> Result<Self> {
        let Some(song) = client
            .command(commands::CurrentSong)
            .await?
            .map(|song| song.song)
        else {
            return Ok(SongInfo {
                state: PlayState::Stopped,
                tags: HashMap::default(),
                album_art: None,
            });
        };

        Ok(SongInfo {
            state: client.command(commands::Status).await?.state,
            tags: song.tags,
            album_art: get_image(client, &song.url).await?,
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
            "title" => self.single_tag_value(&Tag::Title),
            "album" => self.single_tag_value(&Tag::Album),
            "artist" => self.single_tag_value(&Tag::Artist),
            "albumartist" => self.single_tag_value(&Tag::AlbumArtist),
            _ => Some(token),
        }
        .unwrap_or_default()
        .to_string()
    }

    pub fn to_notification(&self) -> Notification {
        let (summary, body) = match self.state {
            PlayState::Stopped => (
                format_notification_text(&CONFIG.stopped_text.summary, self),
                format_notification_text(&CONFIG.stopped_text.body, self),
            ),
            PlayState::Playing => (
                format_notification_text(&CONFIG.playing_text.summary, self),
                format_notification_text(&CONFIG.playing_text.body, self),
            ),
            PlayState::Paused => (
                format_notification_text(&CONFIG.paused_text.summary, self),
                format_notification_text(&CONFIG.paused_text.body, self),
            ),
        };

        Notification::new().summary(&summary).body(&body).finalize()
    }
}
