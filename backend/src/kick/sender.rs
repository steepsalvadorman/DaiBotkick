use crate::state::{AppState, ChannelState};
use std::sync::Arc;

pub async fn send(text: &str, ch: &Arc<ChannelState>, global: &Arc<AppState>) {
    let Some(id) = *ch.channel_id.read().await else {
        return;
    };
    remember(ch, text).await;
    for attempt in 0..2 {
        let token = ch.access_token.read().await.clone();
        let response = global
            .http
            .post(format!("{}/chat", global.kick_endpoints.api))
            .bearer_auth(&token)
            .json(&serde_json::json!({"broadcaster_user_id":id,"content":text,"type":"user"}))
            .send()
            .await;
        match response {
            Ok(r) if r.status().is_success() => {
                tracing::debug!("[Chat][{}] Mensaje enviado", ch.slug);
                return;
            }
            Ok(r) if r.status() == reqwest::StatusCode::UNAUTHORIZED && attempt == 0 => {
                if !super::refresh_if_current(ch, global, &token).await {
                    return;
                }
            }
            Ok(r) => {
                tracing::warn!("[Chat][{}] HTTP {}", ch.slug, r.status());
                return;
            }
            Err(_) => {
                tracing::warn!("[Chat][{}] Error de red", ch.slug);
                return;
            }
        }
    }
}

const RECENT_LIMIT: usize = 20;

/// Guarda el texto para reconocer su eco cuando Kick lo devuelva como mensaje del canal.
async fn remember(ch: &ChannelState, text: &str) {
    let mut recent = ch.recent_sent.lock().await;
    if recent.len() >= RECENT_LIMIT {
        recent.pop_front();
    }
    recent.push_back(text.trim().to_owned());
}

/// `true` si `content` es un mensaje enviado por el bot (y lo consume).
pub async fn is_own_echo(ch: &ChannelState, content: &str) -> bool {
    let mut recent = ch.recent_sent.lock().await;
    match recent.iter().position(|t| t == content.trim()) {
        Some(i) => {
            recent.remove(i);
            true
        }
        None => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn own_messages_are_recognized_once_and_others_are_not() {
        let (ch, _rx) = crate::test_support::channel("alpha", 1);
        remember(&ch, "▶ @ana agregó «x» a la cola").await;
        assert!(is_own_echo(&ch, "▶ @ana agregó «x» a la cola ").await);
        assert!(!is_own_echo(&ch, "▶ @ana agregó «x» a la cola").await);
        assert!(!is_own_echo(&ch, "hola chat").await);
        for i in 0..(RECENT_LIMIT + 5) {
            remember(&ch, &format!("m{i}")).await;
        }
        assert_eq!(ch.recent_sent.lock().await.len(), RECENT_LIMIT);
    }
}
