//! Salida al chat: cada canal responde en su propia plataforma.

use crate::{
    platform::{is_blocked, ChatMessage, Platform, Role},
    server::ns_emit,
    state::{AppState, ChannelState},
    tts,
};
use std::sync::Arc;

/// Publica `text` en el chat del canal como el bot de su plataforma.
pub async fn send(text: &str, ch: &Arc<ChannelState>, global: &Arc<AppState>) {
    match ch.platform {
        Platform::Kick => crate::kick::sender::send(text, ch, global).await,
        Platform::Twitch => crate::twitch::sender::send(text, ch, global).await,
    }
}

/// Mensaje de chat ya verificado (y que no es eco del bot), de cualquier plataforma:
/// lo muestra en el dock y el overlay y ejecuta sus comandos.
pub async fn receive(app: &Arc<AppState>, ch: &Arc<ChannelState>, msg: &ChatMessage) {
    // Un baneado (incluso si es moderador) solo aparece como texto en el chat del overlay.
    let banned = is_blocked(ch, &msg.username, msg.role).await;
    // El dock de chat de OBS ve todo, comandos incluidos, con color e insignias.
    ns_emit(
        app,
        ch,
        "dockChat",
        serde_json::json!({
            "user": msg.username,
            "content": msg.text,
            "textOnly": banned,
            "color": msg.color,
            "badges": msg.badges,
        }),
    );
    // Los comandos (!play, !s…) no se muestran en el chat del overlay.
    if !msg.text.starts_with('!') {
        ns_emit(
            app,
            ch,
            "chatMessage",
            serde_json::json!({"user":msg.username,"content":msg.text,"textOnly":banned}),
        );
    }
    if !banned {
        crate::commands::handle(msg, ch, app).await;
    }
}

/// Alerta y voz de un nuevo seguidor. Un baneado no la dispara siguiendo y dejando de seguir.
pub async fn followed(app: &AppState, ch: &Arc<ChannelState>, username: &str, role: Role) {
    if is_blocked(ch, username, role).await {
        return;
    }
    let message = format!("¡Gracias por el follow, {username}!");
    tts::enqueue(ch, &message, "dalia");
    ns_emit(
        app,
        ch,
        "kickAlert",
        serde_json::json!({"type":"follow","username":username,"message":message}),
    );
}

pub fn subscribed(app: &AppState, ch: &Arc<ChannelState>, username: &str, months: u64) {
    let message = format!("¡{username} se suscribió por {months} meses!");
    tts::enqueue(ch, &message, "dalia");
    ns_emit(
        app,
        ch,
        "kickAlert",
        serde_json::json!({"type":"sub","username":username,"months":months,"message":message}),
    );
}

pub fn gifted(app: &AppState, ch: &Arc<ChannelState>, gifter: &str, count: u64) {
    let message = format!("¡{gifter} regaló {count} suscripciones!");
    tts::enqueue(ch, &message, "dalia");
    ns_emit(
        app,
        ch,
        "kickAlert",
        serde_json::json!({"type":"giftsub","message":message}),
    );
}

/// Inicio (`Some`) o fin (`None`) del directo; `!uptime` cuenta desde `started_at`.
pub async fn live_status(
    app: &AppState,
    ch: &ChannelState,
    started_at: Option<chrono::DateTime<chrono::Utc>>,
    live: bool,
) {
    *ch.live_since.write().await = if live { started_at } else { None };
    ns_emit(
        app,
        ch,
        "streamStatus",
        serde_json::json!({"live":live,"startedAt":ch.live_since.read().await.map(|t| t.to_rfc3339())}),
    );
}

/// Fecha RFC 3339 de un evento, en UTC.
pub fn parse_time(value: &serde_json::Value) -> Option<chrono::DateTime<chrono::Utc>> {
    value
        .as_str()
        .and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok())
        .map(|t| t.with_timezone(&chrono::Utc))
}
