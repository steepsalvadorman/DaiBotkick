use crate::state::{AppState, ChannelState};
use std::sync::Arc;

pub async fn send(text: &str, ch: &Arc<ChannelState>, global: &Arc<AppState>) {
    let Some(id) = *ch.channel_id.read().await else {
        return;
    };
    for attempt in 0..2 {
        let token = ch.access_token.read().await.clone();
        let response = global
            .http
            .post(format!("{}/chat", global.kick_endpoints.api))
            .bearer_auth(token)
            .json(&serde_json::json!({"broadcaster_user_id":id,"content":text,"type":"user"}))
            .send()
            .await;
        match response {
            Ok(r) if r.status().is_success() => {
                tracing::debug!("[Chat][{}] Mensaje enviado", ch.slug);
                return;
            }
            Ok(r) if r.status() == reqwest::StatusCode::UNAUTHORIZED && attempt == 0 => {
                if !super::refresh_access_token(ch, global).await {
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
