use crate::{
    config::CONFIG,
    mpd::{self, SongInfo, SongUpdate},
    utils::{self, open_in_browser},
};
use anyhow::Result;
use md5::{Digest, Md5};
use mpd_client::{responses::PlayState, tag::Tag};
use reqwest::{Client, StatusCode};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{
    fmt::Display,
    fs::{self, File, OpenOptions},
    io::Write,
    mem,
    time::Duration,
};
use tokio::sync::{
    mpsc::{self, Receiver, Sender},
    oneshot,
};
use tracing::{error, info};

pub enum ScrobbleEvent {
    Update((SongInfo, SongUpdate)),
    Shutdown(oneshot::Sender<()>),
}

pub fn spawn() -> Option<Sender<ScrobbleEvent>> {
    if !CONFIG.scrobbling.lastfm.as_ref().is_some_and(|c| c.enable) {
        return None;
    }

    let (scrobbling_tx, scrobbling_rx) = mpsc::channel(16);

    tokio::spawn(async move {
        let mut client = Scrobbling::new().await?;
        client.run(scrobbling_rx).await
    });

    Some(scrobbling_tx)
}

fn song_artist(song: &SongInfo) -> Option<&str> {
    let artist = || song.single_tag_value(&Tag::Artist);
    let album_artist = || song.single_tag_value(&Tag::AlbumArtist);

    if CONFIG
        .scrobbling
        .lastfm
        .as_ref()
        .is_some_and(|c| c.prefer_album_artist)
    {
        album_artist().or_else(artist)
    } else {
        artist().or_else(album_artist)
    }
}

fn default_state() -> PlayState {
    PlayState::Paused
}

#[derive(Serialize, Deserialize)]
struct Scrobble {
    #[serde(skip, default = "default_state")]
    state: PlayState,
    artist: Option<String>,
    track: Option<String>,
    album: Option<String>,
    album_artist: Option<String>,
    duration: Option<u64>,
    #[serde(skip)]
    fired_at: u64,
    timestamp: u64,
    played_duration: u64,
}

impl Scrobble {
    fn new(song: SongInfo) -> Self {
        Self {
            state: song.state,
            artist: song_artist(&song).map(ToOwned::to_owned),
            track: song.single_tag_value(&Tag::Title).map(ToOwned::to_owned),
            album: song.single_tag_value(&Tag::Album).map(ToOwned::to_owned),
            album_artist: song
                .single_tag_value(&Tag::AlbumArtist)
                .map(ToOwned::to_owned),
            duration: song.duration.map(|d| d.as_secs()),
            fired_at: song.fired_at,
            timestamp: utils::now(),
            played_duration: 0,
        }
    }

    fn is_same_song(&self, song: &SongInfo) -> bool {
        self.track.as_deref() == song.single_tag_value(&Tag::Title)
            && self.album.as_deref() == song.single_tag_value(&Tag::Album)
            && self.artist.as_deref() == song_artist(song)
    }

    fn eligible_for_submission(&self) -> bool {
        self.duration.is_none_or(|d| d >= 30)
            && mpd::playtime_threshold_reached(self.duration, self.played_duration)
    }

    fn update_song(&mut self, song: &SongInfo) {
        self.state = song.state;
        self.fired_at = song.fired_at;
    }

    fn update_duration(&mut self, song: &SongInfo) {
        if self.state == PlayState::Playing {
            self.played_duration += song.fired_at.saturating_sub(self.fired_at);
        }
    }

    fn on_state_change(&mut self, song: SongInfo) {
        self.update_duration(&song);
        self.update_song(&song);
    }

    fn replace(&mut self, song: SongInfo) -> Self {
        self.update_duration(&song);
        mem::replace(self, Scrobble::new(song))
    }
}

#[derive(Default, Serialize, Deserialize)]
struct ScrobbleState {
    current: Option<Scrobble>,
    pending: Vec<Scrobble>,
}

impl ScrobbleState {
    fn read_from_disk() -> Option<Self> {
        let mut path = utils::get_state_dir().ok()?.join("scrobble_state");
        path.set_extension("json");

        let scrobble = fs::read(&path)
            .ok()
            .and_then(|s| serde_json::from_slice(&s).ok());

        std::fs::remove_file(path).ok()?;

        scrobble
    }

    fn save_to_disk(&mut self) -> Result<()> {
        if self
            .current
            .as_ref()
            .is_some_and(|scrobble| scrobble.state == PlayState::Stopped)
        {
            self.current = None;

            if self.pending.is_empty() {
                return Ok(());
            }
        }

        if let Some(scrobble) = &mut self.current
            && scrobble.state == PlayState::Playing
        {
            scrobble.played_duration += utils::now().saturating_sub(scrobble.fired_at);
        }

        let mut path = utils::get_state_dir()?.join("scrobble_state");
        path.set_extension("json");

        let state = serde_json::to_string_pretty(self)?;

        let mut file = File::create(path)?;
        file.write_all(state.as_bytes())?;

        Ok(())
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

#[derive(PartialEq)]
enum SubmissionDisposition {
    Retry,
    Hold,
    Discard,
}

fn retryable_status(status: StatusCode) -> bool {
    matches!(
        status,
        StatusCode::REQUEST_TIMEOUT
            | StatusCode::TOO_MANY_REQUESTS
            | StatusCode::INTERNAL_SERVER_ERROR
            | StatusCode::BAD_GATEWAY
            | StatusCode::SERVICE_UNAVAILABLE
            | StatusCode::GATEWAY_TIMEOUT
    )
}

#[derive(Debug)]
enum Error {
    Api { code: u64, message: String },
    Http(reqwest::Error),
}

impl Error {
    fn new(response: &Value) -> Option<Self> {
        Some(Self::Api {
            code: response.get("error")?.as_u64()?,
            message: response.get("message")?.as_str()?.to_owned(),
        })
    }

    fn disposition(&self) -> SubmissionDisposition {
        match self {
            Self::Api {
                code: 8 | 11 | 16 | 29,
                ..
            } => SubmissionDisposition::Retry,
            Self::Api {
                code: 4 | 9 | 10 | 26,
                ..
            } => SubmissionDisposition::Hold,
            Self::Http(error)
                if error.is_timeout()
                    || error.is_connect()
                    || error.status().is_some_and(retryable_status) =>
            {
                SubmissionDisposition::Retry
            }
            _ => SubmissionDisposition::Discard,
        }
    }
}

impl From<reqwest::Error> for Error {
    fn from(e: reqwest::Error) -> Self {
        Self::Http(e)
    }
}

impl Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Api { code, message } => write!(f, "{}: {}", code, message),
            Self::Http(error) => error.fmt(f),
        }
    }
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Api { .. } => None,
            Self::Http(error) => Some(error),
        }
    }
}

pub struct Scrobbling {
    client: Client,
    api_key: String,
    api_secret: String,
    user_session: Option<UserSession>,
    auth_blocked: bool,
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
            auth_blocked: false,
        })
    }

    pub async fn run(&mut self, mut rx: Receiver<ScrobbleEvent>) -> Result<()> {
        let mut scrobble = ScrobbleState::read_from_disk().unwrap_or_default();

        if !scrobble.pending.is_empty() {
            self.scrobble(&mut scrobble).await;
        }

        while let Some(event) = rx.recv().await {
            match event {
                ScrobbleEvent::Update((song, song_update)) => {
                    if scrobble.current.is_some() {
                        match song_update {
                            SongUpdate::Initial => {
                                if let Some(current) = &mut scrobble.current
                                    && current.is_same_song(&song)
                                {
                                    self.on_song_toggle(song, current).await;
                                } else {
                                    self.on_song_change(song, &mut scrobble).await;
                                }
                            }
                            SongUpdate::ToggledState => {
                                self.on_song_toggle(song, scrobble.current.as_mut().unwrap())
                                    .await;
                            }
                            SongUpdate::Changed | SongUpdate::Repeated => {
                                self.on_song_change(song, &mut scrobble).await;
                            }
                            SongUpdate::Stopped => {
                                self.on_song_stopped(song, &mut scrobble).await;
                            }
                            SongUpdate::Seeked | SongUpdate::Unchanged => {}
                        }
                    } else {
                        let is_playing = song.state == PlayState::Playing;
                        let s = Scrobble::new(song);

                        if is_playing {
                            self.now_playing(&s).await;
                        }

                        scrobble.current = Some(s);
                    }
                }
                ScrobbleEvent::Shutdown(ack) => {
                    if let Err(e) = scrobble.save_to_disk() {
                        error!("[Last.fm] failed to save scrobble state: {e}");
                    }

                    let _ = ack.send(());
                    break;
                }
            }
        }

        Ok(())
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

    async fn call_method(&self, method: ApiCall) -> Result<Value, Error> {
        const BASE_URL: &str = "https://ws.audioscrobbler.com/2.0/";

        let arguments = method.build(self);

        let request = self.client.post(BASE_URL).form(&arguments);
        let response = request.send().await.map_err(Error::from)?;
        let status_error = response.error_for_status_ref().err();

        match response.json().await {
            Ok(value) => {
                if let Some(e) = Error::new(&value) {
                    Err(e)
                } else {
                    Ok(value)
                }
            }
            Err(e) => Err(status_error.unwrap_or(e).into()),
        }
    }

    fn sign_call(&self, arguments: &[(String, String)]) -> String {
        let mut arguments = arguments.to_owned();
        arguments.sort_by_key(|args| args.0.clone());

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
        let method = ApiCall::new(Method::GetToken);
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
        let method = ApiCall::new(Method::GetSession).arg("token", token);
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

    async fn queue_scrobble(&mut self, scrobble: Scrobble, state: &mut ScrobbleState) {
        if scrobble.eligible_for_submission() {
            state.pending.push(scrobble);
            self.scrobble(state).await;
        }
    }

    async fn on_song_toggle(&mut self, song: SongInfo, scrobble: &mut Scrobble) {
        if song.state == PlayState::Playing {
            self.now_playing(scrobble).await
        }

        scrobble.on_state_change(song);
    }

    async fn on_song_change(&mut self, song: SongInfo, state: &mut ScrobbleState) {
        let Some(current) = state.current.as_mut() else {
            return;
        };

        let previous = current.replace(song);

        if current.state == PlayState::Playing {
            self.now_playing(current).await;
        }

        self.queue_scrobble(previous, state).await;
    }

    async fn on_song_stopped(&mut self, song: SongInfo, state: &mut ScrobbleState) {
        let Some(mut current) = state.current.take() else {
            return;
        };

        current.update_duration(&song);
        self.queue_scrobble(current, state).await;
    }

    async fn scrobble(&mut self, scrobble: &mut ScrobbleState) {
        let Some(session_key) = self.session_key() else {
            self.on_auth_error(Ok(()));
            return;
        };

        while !scrobble.pending.is_empty() {
            let batch_size = scrobble.pending.len().min(50);

            let method = ApiCall::new(Method::Scrobble)
                .add_songs(&scrobble.pending[0..batch_size])
                .arg("sk", session_key);

            match self.call_method(method).await {
                Ok(response) => {
                    info!("[Last.fm] scrobble response: {response}");
                    scrobble.pending.drain(..batch_size);
                }
                Err(e) => {
                    error!("[Last.fm] failed to scrobble: {e}");
                    match e.disposition() {
                        SubmissionDisposition::Retry => {}
                        SubmissionDisposition::Hold => {
                            self.on_auth_error(Err(e));
                        }
                        SubmissionDisposition::Discard => {
                            scrobble.pending.drain(..batch_size);
                        }
                    }

                    return;
                }
            }
        }
    }

    async fn now_playing(&mut self, scrobble: &Scrobble) {
        let Some(session_key) = self.session_key() else {
            self.on_auth_error(Ok(()));
            return;
        };

        let method = ApiCall::new(Method::UpdateNowPlaying)
            .add_song(scrobble)
            .arg("sk", session_key);

        match self.call_method(method).await {
            Ok(response) => info!("[Last.fm] now playing response: {response}"),
            Err(e) => {
                error!("[Last.fm] failed to update now playing: {e}");
                if e.disposition() == SubmissionDisposition::Hold {
                    self.on_auth_error(Err(e));
                }
            }
        }
    }

    fn on_auth_error(&mut self, e: Result<(), Error>) {
        self.user_session = None;

        if !self.auth_blocked && e.is_ok() {
            error!(
                "[Last.fm] session key missing; run `{} authenticate`",
                env!("CARGO_PKG_NAME")
            );
        }

        self.auth_blocked = true;
    }
}

#[derive(Copy, Clone, PartialEq)]
enum Method {
    GetToken,
    GetSession,
    Scrobble,
    UpdateNowPlaying,
}

impl Method {
    fn need_signing(&self) -> bool {
        match self {
            Method::GetToken => false,
            Method::GetSession | Method::Scrobble | Method::UpdateNowPlaying => true,
        }
    }
}

impl Display for Method {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{}",
            match self {
                Method::GetToken => "auth.getToken",
                Method::GetSession => "auth.getSession",
                Method::Scrobble => "track.scrobble",
                Method::UpdateNowPlaying => "track.updateNowPlaying",
            }
        )
    }
}

struct ApiCall {
    method: Method,
    args: Vec<(String, String)>,
}

impl ApiCall {
    fn new(method: Method) -> Self {
        Self {
            method,
            args: Vec::new(),
        }
    }

    fn build(self, client: &Scrobbling) -> Vec<(String, String)> {
        let method = self.method.to_string();
        let mut call = self
            .arg("method", method)
            .arg("api_key", client.api_key.clone());

        if call.method.need_signing() {
            let api_sig = client.sign_call(&call.args);
            call = call.arg("api_sig", api_sig);
        }

        call.arg("format", "json".to_string()).args
    }

    fn add_song(self, scrobble: &Scrobble) -> Self {
        let mut m = self
            .tag_value("artist", scrobble.artist.as_deref())
            .tag_value("track", scrobble.track.as_deref())
            .tag_value("album", scrobble.album.as_deref())
            .tag_value("albumArtist", scrobble.album_artist.as_deref())
            .arg("duration", scrobble.duration.unwrap_or_default());

        if m.method == Method::Scrobble {
            m = m.arg("timestamp", scrobble.timestamp);
        }

        m
    }

    fn add_songs(self, scrobbles: &[Scrobble]) -> Self {
        if scrobbles.len() <= 1
            && let Some(scrobble) = scrobbles.first()
        {
            self.add_song(scrobble)
        } else {
            let mut m = self;
            for (idx, scrobble) in scrobbles.iter().enumerate() {
                m = m
                    .tag_value(format!("artist[{idx}]"), scrobble.artist.as_deref())
                    .tag_value(format!("track[{idx}]"), scrobble.track.as_deref())
                    .tag_value(format!("album[{idx}]"), scrobble.album.as_deref())
                    .tag_value(
                        format!("albumArtist[{idx}]"),
                        scrobble.album_artist.as_deref(),
                    )
                    .arg(
                        format!("duration[{idx}]"),
                        scrobble.duration.unwrap_or_default(),
                    );

                if m.method == Method::Scrobble {
                    m = m.arg(format!("timestamp[{idx}]"), scrobble.timestamp);
                }
            }

            m
        }
    }

    fn arg<T: ToString, V: ToString>(mut self, key: T, value: V) -> Self {
        self.args.push((key.to_string(), value.to_string()));
        self
    }

    fn tag_value<T: ToString>(mut self, key: T, value: Option<&str>) -> Self {
        if let Some(value) = value {
            self.args.push((key.to_string(), value.to_owned()));
        }

        self
    }
}
