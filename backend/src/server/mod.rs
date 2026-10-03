use crate::{
    commands, queue,
    state::{AppState, ChannelState},
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
    io.ns("/", move |socket: SocketRef, TryData(auth): TryData<Auth>| {
        let global = global.clone();
        async move {
            let slug = socket.req_parts().uri.query().and_then(|q| url::form_urlencoded::parse(q.as_bytes())
                .find(|(k, _)| k == "ch").map(|(_, v)| v.into_owned())).unwrap_or_default();
            let ch = global.channels.get(&slug).map(|c| c.clone());
            let Some(ch) = ch else { socket.emit("channelError", "Canal no disponible").ok(); return; };
            socket.join(slug.clone()).ok();
            let token = auth.ok().and_then(|a| a.token);
            let player_ok = authorized(&ch.playback_token, token.as_deref());
            let socket_id = socket.id.to_string();
            if player_ok {
                let mut player = ch.player.lock().await;
                if player.is_none() { *player = Some(socket_id.clone()); }
            }
            let active = *ch.player.lock().await == Some(socket_id.clone());
            socket.emit("playbackRole", serde_json::json!({"active":active})).ok();
            socket.emit("config", serde_json::json!({"channel_name":slug,"kick_url":format!("kick.com/{slug}")})).ok();
            socket.emit("toggleVideo", serde_json::json!({"showVideo":ch.show_video.load(Ordering::Relaxed)})).ok();
            let q = ch.video_queue.read().await;
            socket.emit("syncQueue", serde_json::json!({"items":q.items,"version":q.version})).ok();
            drop(q);
            let followers = ch.followers_known.load(Ordering::Relaxed).then(|| ch.followers.load(Ordering::Relaxed));
            socket.emit("followersUpdate", serde_json::json!({"count":followers})).ok();
            let (dc, dg) = (ch.clone(), global.clone());
            socket.on_disconnect(move |s: SocketRef, _: socketioxide::socket::DisconnectReason| {
                let (ch, app) = (dc.clone(), dg.clone());
                async move {
                    let mut player = ch.player.lock().await;
                    if *player == Some(s.id.to_string()) {
                        *player = None;
                        ns_emit(&app, &ch.slug, "playerAvailable", serde_json::json!({}));
                    }
                }
            });
            let (ac, ag) = (ch.clone(), global.clone());
            socket.on("advanceQueue", move |s: SocketRef, Data(data): Data<AdvanceCmd>| {
                let (ch, app) = (ac.clone(), ag.clone());
                async move {
                    if ch.cancel.is_cancelled() || *ch.player.lock().await != Some(s.id.to_string()) { return; }
                    if let Err(e) = queue::change(&ch, &app, |q| q.advance_if(&data.id, data.version)).await {
                        tracing::error!("[Queue][{}] Advance: {e}", ch.slug);
                        s.emit("operationError", "No se pudo guardar el avance").ok();
                    }
                    let q = ch.video_queue.read().await;
                    s.emit("syncQueue", serde_json::json!({"items":q.items,"version":q.version})).ok();
                }
            });
            let (pc, pg) = (ch.clone(), global.clone());
            socket.on("panelCommand", move |s: SocketRef, Data(data): Data<PanelCmd>| {
                let (ch, app) = (pc.clone(), pg.clone());
                async move {
                    if ch.cancel.is_cancelled() || !authorized(&ch.panel_token, data.token.as_deref()) { return; }
                    let result = match data.command.as_str() {
                        "play" => { if let Some(url) = data.args { commands::play(url, "panel".into(), &ch, &app).await; } Ok(true) }
                        "tts" => { if let Some(text) = data.args { tts::enqueue(&ch, &text, data.voice.as_deref().unwrap_or("dalia")); } Ok(true) }
                        "skip" => queue::change(&ch, &app, |q| { if q.items.is_empty() { return false; } q.advance(); true }).await,
                        "clearQueue" => queue::change(&ch, &app, |q| { q.clear(); true }).await,
                        "removeFromQueue" => queue::change(&ch, &app, |q| { let Some(index) = data.index.filter(|i| *i < q.items.len()) else { return false; }; q.remove(index); true }).await,
                        "toggleVideo" => set_visibility(&ch, &app, data.show.unwrap_or(true)).await.map(|_| true),
                        _ => Ok(false),
                    };
                    if let Err(e) = result { tracing::error!("[Panel][{}] {e}", ch.slug); s.emit("operationError", "No se pudo guardar el cambio").ok(); }
                }
            });
        }
    });
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
