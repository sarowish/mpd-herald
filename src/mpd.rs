use crate::{
    config::{CONFIG, format_notification_text},
    notification, rpc,
};
use anyhow::Result;
use bytes::BytesMut;
use discord_presence::Client as DiscordClient;
use mpd_client::{
    Client,
    client::{ConnectionEvent, Subsystem},
    commands,
    responses::PlayState,
    tag::Tag,
};
use notify_rust::Notification;
use std::{collections::HashMap, time::Duration};
use tokio::net::TcpStream;

pub async fn connect_to_mpd(mut drpc: DiscordClient) -> Result<()> {
    let connection = TcpStream::connect(format!("{}:{}", CONFIG.host, CONFIG.port)).await?;

    let (client, mut state_changes) = Client::connect(connection).await?;

    let mut song_info = SongInfo::new(&client).await?;
    let mut handle = notification::init(&song_info)?.show()?;
    rpc::update(&mut drpc, song_info).await;

    loop {
        match state_changes.next().await {
            Some(ConnectionEvent::SubsystemChange(Subsystem::Player)) => {
                song_info = SongInfo::new(&client).await?;

                handle = notification::update(&mut handle, &song_info)?;
                rpc::update(&mut drpc, song_info).await;
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
    pub elapsed: Option<Duration>,
    pub duration: Option<Duration>,
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
                elapsed: None,
                duration: None,
                tags: HashMap::default(),
                album_art: None,
            });
        };

        let status = client.command(commands::Status).await?;

        Ok(SongInfo {
            state: status.state,
            elapsed: status.elapsed,
            duration: status.duration,
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
        let notification_config = &CONFIG.notification;

        let (summary, body) = match self.state {
            PlayState::Stopped => (
                format_notification_text(&notification_config.stopped_text.summary, self),
                format_notification_text(&notification_config.stopped_text.body, self),
            ),
            PlayState::Playing => (
                format_notification_text(&notification_config.playing_text.summary, self),
                format_notification_text(&notification_config.playing_text.body, self),
            ),
            PlayState::Paused => (
                format_notification_text(&notification_config.paused_text.summary, self),
                format_notification_text(&notification_config.paused_text.body, self),
            ),
        };

        Notification::new().summary(&summary).body(&body).finalize()
    }
}
