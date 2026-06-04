use crate::{commands, tts, AppState};
use socketioxide::extract::SocketRef;
use std::sync::Arc;
use tracing::info;

/// Payload de advanceQueue: el overlay manda el video que acaba de terminar.
/// El servidor solo avanza si ese video sigue siendo el primero de la cola.
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

pub fn setup(io: &socketioxide::SocketIo, state: Arc<AppState>) {
    io.ns("/", move |socket: SocketRef| {
        let state = state.clone();
        let s2    = socket.clone();

        info!("Widget conectado: {}", socket.id);
        socket.on_disconnect(|s: SocketRef, _: socketioxide::socket::DisconnectReason| async move {
            info!("Widget desconectado: {}", s.id);
        });

        // Enviar config del canal + cola actual al conectar
        let st = state.clone();
        tokio::spawn(async move {
            // Config personalizada (nombre del canal, URL de Kick)
            s2.emit("config", serde_json::json!({
                "channel_name": &st.config.channel_name,
                "kick_url": format!("kick.com/{}", &st.config.channel_name),
            })).ok();

            let q = st.video_queue.read().await;
            s2.emit("syncQueue", serde_json::json!({"items": &q.items})).ok();
        });

        // El overlay avanza la cola cuando termina un video.
        // Solo se procesa si el video que mandó el overlay coincide con el primero
        // de la cola actual — esto previene dobles avances de múltiples overlays.
        let st = state.clone();
        socket.on("advanceQueue", move |_: SocketRef, socketioxide::extract::Data(data): socketioxide::extract::Data<AdvanceCmd>| {
            let st = st.clone();
            async move {
                let mut q = st.video_queue.write().await;
                let current = match q.items.first() {
                    Some(item) => item.clone(),
                    None => return, // cola ya vacía
                };
                // Verificar que el video que terminó sea el actual
                let matches = match (data.video_id.as_deref(), data.url.as_deref(),
                                     current.video_id.as_deref(), current.url.as_deref()) {
                    (Some(d), _, Some(c), _) => d == c,          // comparar por videoId
                    (_, Some(d), _, Some(c)) => d == c,          // comparar por url
                    (None, None, None, None) => true,            // item inválido (ambos null)
                    _ => false,                                   // no coincide → ignorar
                };
                if !matches { return; }
                q.advance();
                let items = q.items.clone();
                drop(q);
                st.io.emit("syncQueue", serde_json::json!({"items": &items})).ok();
            }
        });

        // Comandos desde el panel de control
        let st = state.clone();
        socket.on("panelCommand", move |_: SocketRef, socketioxide::extract::Data(data): socketioxide::extract::Data<PanelCmd>| {
            let st = st.clone();
            async move {
                // Validar token (si PANEL_TOKEN está configurado)
                let panel_token = &st.config.panel_token;
                if !panel_token.is_empty() {
                    if data.token.as_deref() != Some(panel_token.as_str()) {
                        return;
                    }
                }

                match data.command.as_str() {
                    "play" => {
                        if let Some(url) = data.args {
                            commands::play(url, "panel".into(), &st).await;
                        }
                    }
                    "skip" => { st.io.emit("nextVideo", serde_json::json!({})).ok(); }
                    "tts"  => {
                        if let Some(text) = data.args {
                            let voice = data.voice.unwrap_or_else(|| "dalia".into());
                            st.tts_tx.send(tts::TtsQueueItem { text, voice }).ok();
                        }
                    }
                    "toggleVideo" => {
                        st.io.emit("toggleVideo", serde_json::json!({"showVideo": data.show.unwrap_or(true)})).ok();
                    }
                    "removeFromQueue" => {
                        if let Some(idx) = data.index {
                            let mut q = st.video_queue.write().await;
                            q.remove(idx);
                            let items = q.items.clone();
                            drop(q);
                            st.io.emit("syncQueue", serde_json::json!({"items": &items})).ok();
                        }
                    }
                    "clearQueue" => {
                        let mut q = st.video_queue.write().await;
                        q.clear();
                        drop(q);
                        st.io.emit("syncQueue", serde_json::json!({"items": []})).ok();
                    }
                    _ => {}
                }
            }
        });
    });
}
