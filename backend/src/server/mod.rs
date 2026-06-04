use crate::{commands, state::AppState, tts};
use socketioxide::extract::SocketRef;
use std::sync::Arc;
use tracing::info;

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct AdvanceCmd {
    video_id: Option<String>,
    url:      Option<String>,
}

#[derive(serde::Deserialize)]
struct PanelCmd {
    command: String,
    args:    Option<String>,
    voice:   Option<String>,
    show:    Option<bool>,
    index:   Option<usize>,
    token:   Option<String>,
}

/// Registra el único namespace "/" con routing por rooms.
/// Cada socket se une a una room con el slug del canal (via query ?ch=slug).
pub fn setup(io: &socketioxide::SocketIo, global: Arc<AppState>) {
    io.ns("/", move |socket: SocketRef| {
        let global = global.clone();

        // Leer slug del canal desde el query param ?ch=slug
        let slug = socket.req_parts()
            .uri
            .query()
            .and_then(|q| {
                url::form_urlencoded::parse(q.as_bytes())
                    .find(|(k, _)| k == "ch")
                    .map(|(_, v)| v.into_owned())
            })
            .unwrap_or_default();

        if slug.is_empty() {
            return; // overlay sin canal — ignorar
        }

        // Obtener ChannelState para este canal
        let ch = match global.channels.get(&slug).map(|c| c.clone()) {
            Some(c) => c,
            None => return,
        };

        // Unir socket a la room del canal
        socket.join(slug.clone()).ok();

        info!("Widget conectado: {} (canal: {slug})", socket.id);
        let slug2 = slug.clone();
        socket.on_disconnect(move |s: SocketRef, _: socketioxide::socket::DisconnectReason| {
            let slug = slug2.clone();
            async move { info!("Widget desconectado: {} (canal: {slug})", s.id); }
        });

        // Enviar config y cola al conectar
        let ch2 = ch.clone();
        let s2 = socket.clone();
        tokio::spawn(async move {
            s2.emit("config", serde_json::json!({
                "channel_name": &ch2.slug,
                "kick_url":     format!("kick.com/{}", &ch2.slug),
            })).ok();
            let q = ch2.video_queue.read().await;
            s2.emit("syncQueue", serde_json::json!({"items": &q.items})).ok();
        });

        // advanceQueue
        let ch3 = ch.clone();
        let g2  = global.clone();
        socket.on("advanceQueue", move |_: SocketRef, socketioxide::extract::Data(data): socketioxide::extract::Data<AdvanceCmd>| {
            let ch = ch3.clone(); let g = g2.clone();
            async move {
                let mut q = ch.video_queue.write().await;
                let current = match q.items.first() { Some(i) => i.clone(), None => return };
                let matches = match (data.video_id.as_deref(), data.url.as_deref(),
                                     current.video_id.as_deref(), current.url.as_deref()) {
                    (Some(d), _, Some(c), _) => d == c,
                    (_, Some(d), _, Some(c)) => d == c,
                    (None, None, None, None) => true,
                    _ => false,
                };
                if !matches { return; }
                q.advance();
                let items = q.items.clone(); drop(q);
                ns_emit(&g, &ch.slug, "syncQueue", serde_json::json!({"items": &items}));
            }
        });

        // panelCommand
        let ch4 = ch.clone();
        let g3  = global.clone();
        socket.on("panelCommand", move |_: SocketRef, socketioxide::extract::Data(data): socketioxide::extract::Data<PanelCmd>| {
            let ch = ch4.clone(); let g = g3.clone();
            async move {
                if !ch.panel_token.is_empty() && data.token.as_deref() != Some(ch.panel_token.as_str()) { return; }
                match data.command.as_str() {
                    "play" => {
                        if let Some(url) = data.args {
                            commands::play(url, "panel".into(), &ch, &g).await;
                        }
                    }
                    "skip" => ns_emit(&g, &ch.slug, "nextVideo", serde_json::json!({})),
                    "tts"  => {
                        if let Some(text) = data.args {
                            let voice = data.voice.unwrap_or_else(|| "dalia".into());
                            ch.tts_tx.send(tts::TtsQueueItem { text, voice }).ok();
                        }
                    }
                    "toggleVideo" => {
                        ns_emit(&g, &ch.slug, "toggleVideo",
                            serde_json::json!({"showVideo": data.show.unwrap_or(true)}));
                    }
                    "removeFromQueue" => {
                        if let Some(idx) = data.index {
                            let mut q = ch.video_queue.write().await;
                            q.remove(idx);
                            let items = q.items.clone(); drop(q);
                            ns_emit(&g, &ch.slug, "syncQueue", serde_json::json!({"items": &items}));
                        }
                    }
                    "clearQueue" => {
                        ch.video_queue.write().await.clear();
                        ns_emit(&g, &ch.slug, "syncQueue", serde_json::json!({"items": []}));
                    }
                    _ => {}
                }
            }
        });
    });
}

/// Emite a todos los sockets del canal (room = slug).
pub fn ns_emit(global: &AppState, slug: &str, event: &'static str, data: serde_json::Value) {
    global.io.to(slug.to_owned()).emit(event, data).ok();
}
