use crate::{
    config::CONFIG,
    mpd::{SongInfo, SongUpdate},
    utils::{self, open_in_browser},
};
use anyhow::Result;
use md5::{Digest, Md5};
use mpd_client::{responses::PlayState, tag::Tag};
use reqwest::Client;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{
    fmt::Display,
    fs::{self, OpenOptions},
    io::Write,
    mem,
    time::Duration,
};
use tokio::sync::mpsc::{self, Receiver, Sender};
use tracing::{error, info};

pub fn spawn() -> Option<Sender<(SongInfo, SongUpdate)>> {
    if !CONFIG.scrobbling.lastfm.as_ref().is_some_and(|c| c.enable) {
        return None;
    }

    let (scrobbling_tx, scrobbling_rx) = mpsc::channel(16);

    tokio::spawn(async move { run(scrobbling_rx).await });

    Some(scrobbling_tx)
}

pub async fn run(mut rx: Receiver<(SongInfo, SongUpdate)>) -> Result<()> {
    let client = Scrobbling::new().await?;
    let mut scrobble: Option<Scrobble> = None;

    while let Some((song, song_update)) = rx.recv().await {
        if song_update == SongUpdate::Stopped {
            if let Some(mut scrobble) = scrobble.take() {
                scrobble.update_duration(&song);
                client.scrobble(scrobble).await;
            }
        } else if let Some(scrobble) = &mut scrobble {
            match song_update {
                SongUpdate::ToggledState => {
                    if song.state == PlayState::Playing {
                        client.now_playing(scrobble).await
                    }

                    scrobble.on_state_change(song);
                }
                SongUpdate::Changed => {
                    let previous = scrobble.replace(song);
                    client.now_playing(scrobble).await;
                    client.scrobble(previous).await
                }
                SongUpdate::Seeked | SongUpdate::Stopped => (),
            }
        } else {
            let is_playing = song.state == PlayState::Playing;
            let s = Scrobble::new(song);

            if is_playing {
                client.now_playing(&s).await;
            }

            scrobble = Some(s);
        }
    }

    Ok(())
}

struct Scrobble {
    song: SongInfo,
    timestamp: u64,
    played_duration: u64,
}

impl Scrobble {
    fn new(song: SongInfo) -> Self {
        Self {
            song,
            timestamp: utils::now(),
            played_duration: 0,
        }
    }

    fn update_duration(&mut self, song: &SongInfo) {
        if self.song.state == PlayState::Playing {
            self.played_duration += song.fired_at.saturating_sub(self.song.fired_at);
        }
    }

    fn on_state_change(&mut self, song: SongInfo) {
        self.update_duration(&song);
        self.song = song;
    }

    fn replace(&mut self, song: SongInfo) -> Self {
        self.update_duration(&song);
        mem::replace(self, Scrobble::new(song))
    }
}

#[derive(Deserialize, Serialize)]
struct UserSession {
    user: String,
    key: String,
}

impl UserSession {
    fn new(user: &str, session_key: &str) -> Self {
        Self {
            user: user.to_string(),
            key: session_key.to_string(),
        }
    }
}

#[derive(Debug)]
struct ScrobblingError {
    code: u64,
    message: String,
}

impl ScrobblingError {
    fn new(response: &Value) -> Option<Self> {
        Some(Self {
            code: response.get("error")?.as_u64()?,
            message: response.get("message")?.as_str()?.to_owned(),
        })
    }
}

impl Display for ScrobblingError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", self.code, self.message)
    }
}

impl std::error::Error for ScrobblingError {}

pub struct Scrobbling {
    client: Client,
    api_key: String,
    api_secret: String,
    user_session: Option<UserSession>,
}

impl Scrobbling {
    pub async fn new() -> Result<Self> {
        let config = CONFIG
            .scrobbling
            .lastfm
            .as_ref()
            .expect("last.fm configuration isn't provided");

        let client = Client::builder().timeout(Duration::from_secs(5)).build()?;

        let mut path = utils::get_state_dir()?.join("session");
        path.set_extension("toml");

        let user_session = fs::read(path).ok().and_then(|s| toml::from_slice(&s).ok());

        Ok(Self {
            client,
            api_key: config.api_key.clone(),
            api_secret: config.secret.clone(),
            user_session,
        })
    }

    fn session_key(&self) -> Option<&str> {
        self.user_session.as_ref().map(|s| s.key.as_str())
    }

    pub async fn authenticate() -> Result<()> {
        let client = Scrobbling::new().await?;
        let token = client.get_token().await?;

        let url = format!(
            "https://www.last.fm/api/auth/?api_key={}&token={token}",
            client.api_key
        );

        open_in_browser(&url).await?;

        println!("Press enter after granting authorization in the opened web page.");
        let stdin = std::io::stdin();
        stdin.read_line(&mut String::new())?;

        let session = client.get_session(&token).await?;
        let session = toml::to_string(&session)?;

        let state_dir = utils::get_state_dir()?;

        if !state_dir.exists() {
            std::fs::create_dir_all(&state_dir)?;
        }

        let mut path = state_dir.join("session");
        path.set_extension("toml");

        if path.exists() {
            println!(
                "{} already exists. Do you want to overwrite it? [Y/n]",
                path.as_os_str().to_str().unwrap_or("file")
            );
            let stdin = std::io::stdin();
            let mut buf = String::new();
            stdin.read_line(&mut buf)?;
            buf = buf.trim().to_lowercase();

            if !buf.is_empty() && buf != "y" {
                std::process::exit(1);
            }
        }

        let mut open_options = OpenOptions::new();
        open_options.write(true).create(true).truncate(true);

        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            open_options.mode(0o600);
        }

        let mut file = open_options.open(path)?;
        file.write_all(session.as_bytes())?;

        Ok(())
    }

    async fn call_method(&self, method: Method<'_>) -> Result<Value> {
        const BASE_URL: &str = "https://ws.audioscrobbler.com/2.0/";

        let arguments = method.build(self);

        let request = self.client.post(BASE_URL).form(&arguments);
        let response: Value = request.send().await?.error_for_status()?.json().await?;

        if let Some(e) = ScrobblingError::new(&response) {
            Err(e.into())
        } else {
            Ok(response)
        }
    }

    fn sign_call(&self, arguments: &[(&str, String)]) -> String {
        let mut arguments = arguments.to_owned();
        arguments.sort_by_key(|args| args.0);

        let mut hasher = Md5::new();

        for (key, value) in arguments {
            hasher.update(key);
            hasher.update(value);
        }

        hasher.update(&self.api_secret);

        let hash = hasher.finalize();

        base16ct::lower::encode_string(&hash)
    }

    async fn get_token(&self) -> Result<String> {
        let method = Method::new("auth.getToken");
        let response = self.call_method(method).await?;

        response
            .get("token")
            .and_then(|s| s.as_str())
            .map(ToOwned::to_owned)
            .ok_or(anyhow::anyhow!(
                "Last.fm auth.getToken response missing token"
            ))
    }

    async fn get_session(&self, token: &str) -> Result<UserSession> {
        let method = Method::new("auth.getSession")
            .arg("token", token)
            .auth(true);
        let response = self.call_method(method).await?;

        let session = &response["session"];

        let key = session
            .get("key")
            .and_then(|s| s.as_str())
            .ok_or(anyhow::anyhow!(
                "Last.fm auth.getSession response missing session.key"
            ))?;
        let user = session
            .get("name")
            .and_then(|s| s.as_str())
            .ok_or(anyhow::anyhow!(
                "Last.fm auth.getSession response missing session.name"
            ))?;

        Ok(UserSession::new(user, key))
    }

    async fn scrobble(&self, scrobble: Scrobble) {
        if let Some(duration) = scrobble.song.duration
            && scrobble.played_duration < duration.as_secs() / 2
            && scrobble.played_duration < 4 * 60
        {
            return;
        }

        let Some(session_key) = self.session_key() else {
            error!(
                "[Last.fm] session key missing; run `{} authenticate`",
                env!("CARGO_PKG_NAME")
            );
            return;
        };

        let method = Method::new("track.scrobble")
            .tag_value(&scrobble, "artist", Tag::Artist)
            .tag_value(&scrobble, "track", Tag::Title)
            .tag_value(&scrobble, "album", Tag::Album)
            .tag_value(&scrobble, "albumArtist", Tag::AlbumArtist)
            .arg(
                "duration",
                scrobble.song.duration.unwrap_or_default().as_secs(),
            )
            .arg("timestamp", scrobble.timestamp)
            .arg("sk", session_key)
            .auth(true);

        match self.call_method(method).await {
            Ok(response) => info!("[Last.fm] scrobble response: {response}"),
            Err(e) => error!("[Last.fm] failed to scrobble: {e}"),
        }
    }

    async fn now_playing(&self, scrobble: &Scrobble) {
        let Some(session_key) = self.session_key() else {
            error!(
                "[Last.fm] session key missing; run `{} authenticate`",
                env!("CARGO_PKG_NAME")
            );
            return;
        };

        let method = Method::new("track.updateNowPlaying")
            .tag_value(scrobble, "artist", Tag::Artist)
            .tag_value(scrobble, "track", Tag::Title)
            .tag_value(scrobble, "album", Tag::Album)
            .tag_value(scrobble, "albumArtist", Tag::AlbumArtist)
            .arg(
                "duration",
                scrobble.song.duration.unwrap_or_default().as_secs(),
            )
            .arg("sk", session_key)
            .auth(true);

        match self.call_method(method).await {
            Ok(response) => info!("[Last.fm] now playing response: {response}"),
            Err(e) => error!("[Last.fm] failed to update now playing: {e}"),
        }
    }
}

struct Method<'a> {
    name: String,
    args: Vec<(&'a str, String)>,
    auth: bool,
}

impl<'a> Method<'a> {
    fn new(name: &str) -> Self {
        Self {
            name: name.to_owned(),
            args: Vec::new(),
            auth: false,
        }
    }

    fn build(mut self, client: &Scrobbling) -> Vec<(&'a str, String)> {
        self.args.push(("method", self.name));
        self.args.push(("api_key", client.api_key.clone()));

        if self.auth {
            let api_sig = client.sign_call(&self.args);
            self.args.push(("api_sig", api_sig));
        }

        self.args.push(("format", "json".to_string()));

        self.args
    }

    fn auth(mut self, auth: bool) -> Self {
        self.auth = auth;
        self
    }

    fn arg<T: ToString>(mut self, key: &'a str, value: T) -> Self {
        self.args.push((key, value.to_string()));
        self
    }

    fn tag_value(mut self, scrobble: &Scrobble, key: &'a str, tag: Tag) -> Self {
        if let Some(value) = scrobble.song.single_tag_value(&tag) {
            self.args.push((key, value.to_owned()));
        }

        self
    }
}
