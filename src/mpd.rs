use crate::{
    config::CONFIG,
    notification,
    rpc::{self, RpcEvent},
};
use anyhow::Result;
use bytes::BytesMut;
use mpd_client::{
    Client,
    client::{ConnectionEvent, Subsystem},
    commands,
    responses::PlayState,
    tag::Tag,
};
use std::{collections::HashMap, time::Duration};
use tokio::{net::TcpStream, sync::mpsc};

pub async fn connect_to_mpd() -> Result<()> {
    let connection = TcpStream::connect(format!("{}:{}", CONFIG.host, CONFIG.port)).await?;
    let (client, mut mpd_rx) = Client::connect(connection).await?;

    let (tx, rx) = mpsc::channel(16);
    let tx2 = tx.clone();

    tokio::spawn(async move { rpc::run(tx2, rx).await });

    let mut song_info = SongInfo::new(&client).await?;
    let mut handle = notification::init(&client, &song_info).await?.show()?;
    tx.send(RpcEvent::Update(song_info.clone(), false)).await?;

    loop {
        match mpd_rx.next().await {
            Some(ConnectionEvent::SubsystemChange(Subsystem::Player)) => {
                let old_info = std::mem::replace(&mut song_info, SongInfo::new(&client).await?);
                let only_seeked = song_info.only_seeked(&old_info);

                if !only_seeked {
                    notification::update(&mut handle, &client, &song_info).await?;
                }

                tx.send(RpcEvent::Update(song_info.clone(), only_seeked))
                    .await?;
            }
            Some(_) => (),
            None => return Ok(()),
        }
    }
}

pub async fn get_image(client: &Client, uri: &str) -> Result<Option<BytesMut>> {
    let mut out = BytesMut::new();
    let mut expected_size = 0;

    let from_file = if let Some(resp) = client.command(commands::AlbumArt::new(uri)).await? {
        out = resp.data;
        expected_size = resp.size;
        out.reserve(expected_size);

        true
    } else {
        false
    };

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

#[derive(Clone)]
pub struct SongInfo {
    pub url: String,
    pub state: PlayState,
    pub elapsed: Option<Duration>,
    pub duration: Option<Duration>,
    pub tags: HashMap<Tag, Vec<String>>,
}

impl SongInfo {
    async fn new(client: &Client) -> Result<Self> {
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
            });
        };

        let status = client.command(commands::Status).await?;

        Ok(Self {
            url: song.url,
            state: status.state,
            elapsed: status.elapsed,
            duration: status.duration,
            tags: song.tags,
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

    pub fn only_seeked(&self, other: &Self) -> bool {
        self.tags == other.tags && self.state == other.state
    }
}
