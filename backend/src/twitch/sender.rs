//! Respuestas de DaiBotsito en el chat de Twitch (`POST /helix/chat/messages`).
//! Se envían con el token de app: así Twitch muestra la insignia de bot de chat, porque la
//! cuenta bot autorizó `user:bot` y el canal `channel:bot`.

use crate::{
    server::ns_emit,
    state::{AppState, ChannelState},
};
use std::{sync::Arc, time::Duration};

/// Máximo de caracteres de un mensaje de chat en Twitch.
const MESSAGE_MAX: usize = 500;

/// Separación mínima entre mensajes del bot en un canal: 20 cada 30 s, el límite de Twitch
/// para una cuenta que no es moderadora. El bingo puede anunciar varias cosas seguidas.
const MIN_GAP: Duration = Duration::from_millis(1500);

pub async fn send(text: &str, ch: &Arc<ChannelState>, app: &Arc<AppState>) {
    let (Some(tw), Some(broadcaster)) = (&app.config.twitch, *ch.channel_id.read().await) else {
        return;
    };
    let Some(bot_id) = app
        .twitch_bot
        .read()
        .await
        .as_ref()
        .map(|b| b.user_id.clone())
    else {
        tracing::warn!(
            "[Chat][{}] Falta autorizar la cuenta bot en /auth/twitch/bot",
            ch.slug
        );
        return;
    };
    let message = truncate(text);
    let body = serde_json::json!({
        "broadcaster_id": broadcaster.to_string(),
        "sender_id": bot_id,
        "message": message,
    });
    // Un mensaje a la vez por canal y espaciados: conserva el orden y respeta el límite.
    let mut last = ch.last_chat_send.lock().await;
    if let Some(previous) = *last {
        tokio::time::sleep_until((previous + MIN_GAP).into()).await;
    }
    for attempt in 0..2 {
        let Some(token) = super::eventsub::app_token(app).await else {
            tracing::warn!("[Chat][{}] Sin token de app de Twitch", ch.slug);
            return;
        };
        let response = app
            .http
            .post(format!("{}/chat/messages", app.twitch_endpoints.api))
            .bearer_auth(&token)
            .header("Client-Id", &tw.client_id)
            .json(&body)
            .send()
            .await;
        *last = Some(std::time::Instant::now());
        match response {
            Ok(r) if r.status().is_success() => {
                let json: serde_json::Value = r.json().await.unwrap_or_default();
                let sent = &json["data"][0];
                if sent["is_sent"] == true {
                    // El eco no llega al overlay; el dock de chat sí muestra las respuestas del bot.
                    ns_emit(
                        app,
                        ch,
                        "dockChat",
                        serde_json::json!({"user":"DaiBotsito","content":message,"bot":true}),
                    );
                } else {
                    tracing::warn!(
                        "[Chat][{}] Twitch no publicó el mensaje: {}",
                        ch.slug,
                        sent["drop_reason"]["message"]
                            .as_str()
                            .unwrap_or("sin motivo")
                    );
                }
                return;
            }
            // El token de app venció o se revocó: se pide otro y se reintenta una vez.
            Ok(r) if r.status() == reqwest::StatusCode::UNAUTHORIZED && attempt == 0 => {
                *app.twitch_app_token.lock().await = None;
            }
            Ok(r) => {
                let status = r.status();
                let detail: String = r
                    .text()
                    .await
                    .unwrap_or_default()
                    .chars()
                    .take(300)
                    .collect();
                tracing::warn!("[Chat][{}] HTTP {status} {detail}", ch.slug);
                return;
            }
            Err(_) => {
                tracing::warn!("[Chat][{}] Error de red", ch.slug);
                return;
            }
        }
    }
}

/// Corta en 500 caracteres (no bytes) para no partir letras con tilde ni emojis.
fn truncate(text: &str) -> String {
    let text = text.trim();
    match text.char_indices().nth(MESSAGE_MAX) {
        Some((end, _)) => text[..end].to_owned(),
        None => text.to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn long_messages_are_cut_at_500_characters() {
        assert_eq!(truncate("  hola  "), "hola");
        let long = "ñ".repeat(MESSAGE_MAX + 20);
        let cut = truncate(&long);
        assert_eq!(cut.chars().count(), MESSAGE_MAX);
        assert!(long.starts_with(&cut));
    }
}
