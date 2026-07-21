use std::time::Duration;

use crate::{
    config::CONFIG,
    mpd::{self, SongInfo, SongUpdate},
    notification::{self, NotificationEvent, NotificationSender},
    rpc::{self, RpcEvent},
    scrobbling::{self, ScrobbleEvent},
};
use anyhow::Result;
use mpd_client::{
    Client as MpdClient,
    client::{ConnectionEvent, Subsystem},
    responses::PlayState,
};
use tokio::{
    sync::{mpsc::Sender, oneshot},
    time::sleep,
};
use tracing::{error, info};

pub struct Service {
    notification_tx: Option<NotificationSender>,
    rpc_tx: Option<Sender<RpcEvent>>,
    scrobble_tx: Option<Sender<ScrobbleEvent>>,
}

impl Service {
    pub async fn new() -> Result<Self> {
        Ok(Self {
            notification_tx: notification::spawn(),
            rpc_tx: rpc::spawn(),
            scrobble_tx: scrobbling::spawn(),
        })
    }

    pub async fn run(&mut self) -> Result<()> {
        let host = format!("{}:{}", CONFIG.host, CONFIG.port);

        loop {
            let connection = mpd::connect(&host).await;

            match connection {
                Ok((mpd_client, mut mpd_rx)) => {
                    info!("Connected to {host}");

                    let mut song_info = SongInfo::new(&mpd_client).await?;
                    info!("[MPD] {song_info}");

                    self.send_notification_update(&mpd_client, &song_info)?;
                    self.send_rpc_update(&song_info, false).await?;

                    if song_info.state != PlayState::Stopped {
                        self.send_scrobbling_update(&song_info, SongUpdate::Initial)
                            .await?;
                    }

                    while let Some(event) = mpd_rx.next().await {
                        if let ConnectionEvent::SubsystemChange(
                            Subsystem::Player | Subsystem::Queue,
                        ) = event
                        {
                            info!("[MPD] Detected player event");

                            let old_info = std::mem::replace(
                                &mut song_info,
                                SongInfo::new(&mpd_client).await?,
                            );
                            info!("[MPD] {song_info}");

                            let song_update = song_info.check_update(&old_info);
                            let only_seeked = song_update == SongUpdate::Seeked;

                            if !only_seeked {
                                self.send_notification_update(&mpd_client, &song_info)?;
                            }

                            self.send_rpc_update(&song_info, only_seeked).await?;
                            self.send_scrobbling_update(&song_info, song_update).await?;
                        }
                    }

                    if song_info.state == PlayState::Playing {
                        song_info.set_as_paused();

                        self.send_rpc_update(&song_info, false).await?;
                        self.send_scrobbling_update(&song_info, SongUpdate::ToggledState)
                            .await?;
                    }
                }
                Err(e) => error!("Failed to connect to `{host}`: {e}"),
            }

            sleep(Duration::from_secs(5)).await;
        }
    }

    pub async fn shutdown(&mut self) -> Result<()> {
        if let Some(tx) = self.scrobble_tx.take() {
            let (ack_tx, ack_rx) = oneshot::channel();
            tx.send(ScrobbleEvent::Shutdown(ack_tx)).await?;
            ack_rx.await?;
        }

        Ok(())
    }

    fn send_notification_update(&self, mpd_client: &MpdClient, song_info: &SongInfo) -> Result<()> {
        if let Some(tx) = &self.notification_tx {
            tx.force_send(NotificationEvent {
                client: mpd_client.clone(),
                song: song_info.clone(),
            })?;
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
            tx.send(ScrobbleEvent::Update((song_info.to_owned(), song_update)))
                .await?;
        }

        Ok(())
    }
}
