use crate::{
    mpd::{self, SongInfo, SongUpdate, get_image},
    notification,
    rpc::{self, RpcEvent},
    scrobbling,
};
use anyhow::Result;
use bytes::BytesMut;
use mpd_client::{
    Client as MpdClient,
    client::{ConnectionEvent, ConnectionEvents, Subsystem},
    responses::PlayState,
};
use tokio::sync::mpsc::Sender;
use tracing::info;

pub struct Service {
    mpd_client: MpdClient,
    mpd_rx: ConnectionEvents,
    notification_tx: Option<Sender<(SongInfo, Option<BytesMut>)>>,
    rpc_tx: Option<Sender<RpcEvent>>,
    scrobble_tx: Option<Sender<(SongInfo, SongUpdate)>>,
}

impl Service {
    pub async fn new() -> Result<Self> {
        let (mpd_client, mpd_rx) = mpd::connect().await?;

        Ok(Self {
            mpd_client,
            mpd_rx,
            notification_tx: notification::spawn(),
            rpc_tx: rpc::spawn(),
            scrobble_tx: scrobbling::spawn(),
        })
    }

    pub async fn run(&mut self) -> Result<()> {
        let mut song_info = SongInfo::new(&self.mpd_client).await?;
        info!("[MPD] {song_info}");

        self.send_notification_update(&song_info).await?;
        self.send_rpc_update(&song_info, false).await?;

        if song_info.state != PlayState::Stopped {
            self.send_scrobbling_update(&song_info, SongUpdate::Changed)
                .await?;
        }

        loop {
            match self.mpd_rx.next().await {
                Some(ConnectionEvent::SubsystemChange(Subsystem::Player | Subsystem::Queue)) => {
                    info!("[MPD] Detected player event");

                    let old_info =
                        std::mem::replace(&mut song_info, SongInfo::new(&self.mpd_client).await?);
                    info!("[MPD] {song_info}");

                    let song_update = song_info.check_update(&old_info);
                    let only_seeked = song_update == SongUpdate::Seeked;

                    if !only_seeked {
                        self.send_notification_update(&song_info).await?;
                    }

                    self.send_rpc_update(&song_info, only_seeked).await?;
                    self.send_scrobbling_update(&song_info, song_update).await?;
                }
                Some(_) => (),
                None => return Ok(()),
            }
        }
    }

    async fn send_notification_update(&self, song_info: &SongInfo) -> Result<()> {
        if let Some(tx) = &self.notification_tx {
            let art = get_image(&self.mpd_client, &song_info.url)
                .await
                .ok()
                .flatten();
            tx.send((song_info.to_owned(), art)).await?;
        }

        Ok(())
    }

    async fn send_rpc_update(&self, song_info: &SongInfo, seek_only: bool) -> Result<()> {
        if let Some(tx) = &self.rpc_tx {
            tx.send(RpcEvent::Update(song_info.clone(), seek_only))
                .await?;
        }

        Ok(())
    }

    async fn send_scrobbling_update(
        &self,
        song_info: &SongInfo,
        song_update: SongUpdate,
    ) -> Result<()> {
        if let Some(tx) = &self.scrobble_tx {
            tx.send((song_info.to_owned(), song_update)).await?;
        }

        Ok(())
    }
}
