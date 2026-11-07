use crate::{
    commands, queue,
    state::{AppState, ChannelState, PlayerSlot},
    tts,
};
use socketioxide::extract::{Data, SocketRef, TryData};
use std::sync::{atomic::Ordering, Arc};

#[derive(serde::Deserialize)]
struct Auth {
    token: Option<String>,
}
#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct AdvanceCmd {
    id: String,
    version: u64,
}
#[derive(serde::Deserialize)]
struct PingCmd {
    epoch: Option<u64>,
}
#[derive(serde::Deserialize)]
struct PanelCmd {
    command: String,
    args: Option<String>,
    voice: Option<String>,
    show: Option<bool>,
    index: Option<usize>,
    token: Option<String>,
}

pub fn authorized(expected: &str, supplied: Option<&str>) -> bool {
    !expected.is_empty() && supplied == Some(expected)
}

pub fn setup(io: &socketioxide::SocketIo, global: Arc<AppState>) {
    io.ns(
        "/",
        move |socket: SocketRef, TryData(auth): TryData<Auth>| {
            let global = global.clone();
            async move {
                let slug = socket
                    .req_parts()
                    .uri
                    .query()
                    .and_then(|q| {
                        url::form_urlencoded::parse(q.as_bytes())
                            .find(|(k, _)| k == "ch")
                            .map(|(_, v)| v.into_owned())
                    })
                    .unwrap_or_default();
                let ch = global.channels.get(&slug).map(|c| c.clone());
                let Some(ch) = ch else {
                    socket.emit("channelError", "Canal no disponible").ok();
                    return;
                };
                socket.join(slug.clone()).ok();
                let token = auth.ok().and_then(|a| a.token);
                let player_ok = authorized(&ch.playback_token, token.as_deref());
                let socket_id = socket.id.to_string();
                if player_ok {
                    let mut player = ch.player.lock().await;
                    if player.is_none() {
                        let epoch = ch.player_epoch.fetch_add(1, Ordering::Relaxed) + 1;
                        *player = Some(PlayerSlot {
                            socket_id: socket_id.clone(),
                            epoch,
                        });
                    }
                }
                let slot = ch.player.lock().await;
                let active = slot.as_ref().is_some_and(|p| p.socket_id == socket_id);
                let epoch = slot
                    .as_ref()
                    .filter(|p| p.socket_id == socket_id)
                    .map(|p| p.epoch);
                drop(slot);
                socket
                    .emit(
                        "playbackRole",
                        serde_json::json!({"active":active,"epoch":epoch}),
                    )
                    .ok();
                socket.emit("config", ch.overlay_config().await).ok();
                socket
                    .emit(
                        "stageLights",
                        serde_json::json!({"on":ch.stage_lights.load(Ordering::Relaxed)}),
                    )
                    .ok();
                socket
                    .emit(
                        "toggleVideo",
                        serde_json::json!({"showVideo":ch.show_video.load(Ordering::Relaxed)}),
                    )
                    .ok();
                let q = ch.video_queue.read().await;
                socket
                    .emit(
                        "syncQueue",
                        serde_json::json!({"items":q.items,"version":q.version}),
                    )
                    .ok();
                drop(q);
                let bingo = ch.bingo.lock().await;
                socket.emit("bingoState", bingo.snapshot()).ok();
                drop(bingo);
                let followers = ch
                    .followers_known
                    .load(Ordering::Relaxed)
                    .then(|| ch.followers.load(Ordering::Relaxed));
                socket
                    .emit("followersUpdate", serde_json::json!({"count":followers}))
                    .ok();
                let (dc, dg) = (ch.clone(), global.clone());
                socket.on_disconnect(
                    move |s: SocketRef, _: socketioxide::socket::DisconnectReason| {
                        let (ch, app) = (dc.clone(), dg.clone());
                        async move {
                            let mut player = ch.player.lock().await;
                            if player
                                .as_ref()
                                .is_some_and(|p| p.socket_id == s.id.to_string())
                            {
                                *player = None;
                                drop(player);
                                ns_emit(&app, &ch.slug, "playerAvailable", serde_json::json!({}));
                            }
                        }
                    },
                );
                let (ac, ag) = (ch.clone(), global.clone());
                socket.on(
                    "advanceQueue",
                    move |s: SocketRef, Data(data): Data<AdvanceCmd>| {
                        let (ch, app) = (ac.clone(), ag.clone());
                        async move {
                            if ch.cancel.is_cancelled()
                                || !ch
                                    .player
                                    .lock()
                                    .await
                                    .as_ref()
                                    .is_some_and(|p| p.socket_id == s.id.to_string())
                            {
                                return;
                            }
                            if let Err(e) =
                                queue::change(&ch, &app, |q| q.advance_if(&data.id, data.version))
                                    .await
                            {
                                tracing::error!("[Queue][{}] Advance: {e}", ch.slug);
                                s.emit("operationError", "No se pudo guardar el avance")
                                    .ok();
                            }
                            let q = ch.video_queue.read().await;
                            s.emit(
                                "syncQueue",
                                serde_json::json!({"items":q.items,"version":q.version}),
                            )
                            .ok();
                        }
                    },
                );
                // Heartbeat del player: una página desplazada (el slot pasó a otra) se entera
                // acá y corta el audio enseguida, en vez de seguir sonando hasta su propio timeout.
                if player_ok {
                    let hc = ch.clone();
                    socket.on(
                        "playerPing",
                        move |s: SocketRef, Data(data): Data<PingCmd>| {
                            let ch = hc.clone();
                            async move {
                                let slot = ch.player.lock().await;
                                let active = slot.as_ref().is_some_and(|p| {
                                    p.socket_id == s.id.to_string() && Some(p.epoch) == data.epoch
                                });
                                let epoch = slot.as_ref().map(|p| p.epoch);
                                drop(slot);
                                s.emit(
                                    "playerPong",
                                    serde_json::json!({"active":active,"epoch":epoch}),
                                )
                                .ok();
                            }
                        },
                    );
                }
                let (pc, pg) = (ch.clone(), global.clone());
                socket.on(
                    "panelCommand",
                    move |s: SocketRef, Data(data): Data<PanelCmd>| {
                        let (ch, app) = (pc.clone(), pg.clone());
                        async move {
                            if ch.cancel.is_cancelled()
                                || !authorized(&ch.panel_token, data.token.as_deref())
                            {
                                return;
                            }
                            let result = match data.command.as_str() {
                                "play" => {
                                    if let Some(url) = data.args {
                                        commands::play(url, "panel".into(), &ch, &app).await;
                                    }
                                    Ok(true)
                                }
                                "tts" => {
                                    if let Some(text) = data.args {
                                        tts::enqueue(
                                            &ch,
                                            &text,
                                            data.voice.as_deref().unwrap_or("dalia"),
                                        );
                                    }
                                    Ok(true)
                                }
                                "skip" => {
                                    queue::change(&ch, &app, |q| {
                                        if q.items.is_empty() {
                                            return false;
                                        }
                                        q.advance();
                                        true
                                    })
                                    .await
                                }
                                "clearQueue" => {
                                    queue::change(&ch, &app, |q| {
                                        q.clear();
                                        true
                                    })
                                    .await
                                }
                                "removeFromQueue" => {
                                    queue::change(&ch, &app, |q| {
                                        let Some(index) = data.index.filter(|i| *i < q.items.len())
                                        else {
                                            return false;
                                        };
                                        q.remove(index);
                                        true
                                    })
                                    .await
                                }
                                "toggleVideo" => {
                                    set_visibility(&ch, &app, data.show.unwrap_or(true))
                                        .await
                                        .map(|_| true)
                                }
                                _ => Ok(false),
                            };
                            if let Err(e) = result {
                                tracing::error!("[Panel][{}] {e}", ch.slug);
                                s.emit("operationError", "No se pudo guardar el cambio")
                                    .ok();
                            }
                        }
                    },
                );
            }
        },
    );
}

pub async fn set_visibility(
    ch: &ChannelState,
    app: &AppState,
    visible: bool,
) -> Result<(), sqlx::Error> {
    sqlx::query("UPDATE channels SET show_video=$1 WHERE slug=$2")
        .bind(visible)
        .bind(&ch.slug)
        .execute(&app.db)
        .await?;
    ch.show_video.store(visible, Ordering::Relaxed);
    ns_emit(
        app,
        &ch.slug,
        "toggleVideo",
        serde_json::json!({"showVideo":visible}),
    );
    Ok(())
}

pub fn ns_emit(global: &AppState, slug: &str, event: &'static str, data: serde_json::Value) {
    global.io.to(slug.to_owned()).emit(event, data).ok();
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn authorization_fails_closed() {
        assert!(!authorized("", None));
        assert!(!authorized("", Some("")));
        assert!(!authorized("secret", Some("wrong")));
        assert!(!authorized("secret", None));
        assert!(authorized("secret", Some("secret")));
    }
}
