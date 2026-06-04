use crate::state::{AppState, ChannelState};
use std::sync::Arc;
use tracing::{info, warn};

/// Envía un mensaje al chat del canal.
pub async fn send(text: &str, ch: &Arc<ChannelState>, global: &Arc<AppState>) {
    let (channel_id, chatroom_id) = {
        let cid = *ch.channel_id.read().await;
        let crid = *ch.chatroom_id.read().await;
        match (cid, crid) {
            (Some(c), Some(r)) => (c, r),
            _ => { warn!("[Chat] IDs no disponibles para {}", ch.slug); return; }
        }
    };

    let bearer = ch.access_token.read().await.clone();
    if bearer.is_empty() {
        warn!("[Chat] Sin token para {}", ch.slug);
        return;
    }

    match try_send_official(text, channel_id, &bearer, global).await {
        Ok(_)     => return,
        Err(true) => {
            warn!("[Chat] 401 en {slug} — renovando token...", slug = ch.slug);
            if super::refresh_access_token(ch, global).await {
                let new_bearer = ch.access_token.read().await.clone();
                if try_send_official(text, channel_id, &new_bearer, global).await.is_ok() {
                    return;
                }
            }
        }
        Err(false) => {}
    }

    try_send_legacy(text, chatroom_id, &bearer, &ch.slug, global).await;
}

async fn try_send_official(
    text: &str,
    broadcaster_user_id: u64,
    bearer: &str,
    global: &Arc<AppState>,
) -> Result<(), bool> {
    let r = global.http
        .post("https://api.kick.com/public/v1/chat")
        .header("Authorization", format!("Bearer {bearer}"))
        .json(&serde_json::json!({
            "broadcaster_user_id": broadcaster_user_id,
            "content": text,
            "type": "user",
        }))
        .send()
        .await
        .map_err(|e| { warn!("[Chat][official] Red: {e}"); false })?;

    let status = r.status();
    if status.is_success() {
        info!("[Chat] ← \"{}\"", &text[..text.len().min(60)]);
        return Ok(());
    }
    let body = r.text().await.unwrap_or_default();
    warn!("[Chat][official] {status}: {body}");
    Err(status.as_u16() == 401)
}

async fn try_send_legacy(
    text: &str,
    chatroom_id: u64,
    bearer: &str,
    slug: &str,
    global: &Arc<AppState>,
) {
    match global.http
        .post("https://kick.com/api/v2/messages")
        .header("Authorization", format!("Bearer {bearer}"))
        .header("Referer", format!("https://kick.com/{slug}"))
        .header("Origin", "https://kick.com")
        .json(&serde_json::json!({
            "chatroom_id": chatroom_id,
            "content": text,
            "type": "message",
        }))
        .send()
        .await
    {
        Ok(r) if r.status().is_success() => {
            info!("[Chat] ← \"{}\" (legacy)", &text[..text.len().min(60)]);
        }
        Ok(r) => {
            let status = r.status();
            let body = r.text().await.unwrap_or_default();
            warn!("[Chat][legacy] {status}: {body}");
        }
        Err(e) => warn!("[Chat][legacy] Red: {e}"),
    }
}
