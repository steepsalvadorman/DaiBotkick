use tracing::warn;

#[derive(Debug, Clone)]
pub struct ChannelInfo {
    pub channel_id: u64,
    pub chatroom_id: u64,
    pub slug: String,
}

/// Obtiene IDs del canal intentando primero la nueva API pública de Kick
/// y luego la API v1 como fallback.
pub async fn get_channel_info(http: &reqwest::Client, channel: &str, token: &str) -> Option<ChannelInfo> {
    // Intento 1: nueva API pública (requiere OAuth)
    if !token.is_empty() {
        if let Some(info) = try_public_api(http, channel, token).await {
            return Some(info);
        }
    }

    // Intento 2: API v1 clásica
    try_v1_api(http, channel, token).await
}

async fn try_public_api(http: &reqwest::Client, channel: &str, token: &str) -> Option<ChannelInfo> {
    let url = format!("https://api.kick.com/public/v1/channels?broadcaster_username={channel}");
    let resp = http.get(&url)
        .header("Authorization", format!("Bearer {token}"))
        .send()
        .await
        .ok()?;

    let status = resp.status();
    if !status.is_success() {
        let body = resp.text().await.unwrap_or_default();
        warn!("Kick nueva API {channel}: {status} — {body}");
        return None;
    }

    let json: serde_json::Value = resp.json().await.ok()?;

    // La nueva API devuelve { "data": [ { "broadcaster_user_id": ..., ... } ] }
    let ch = json["data"].as_array()?.first()?;
    let channel_id  = ch["broadcaster_user_id"].as_u64()?;
    // chatroom_id puede estar bajo "chatroom" o coincidir con broadcaster_user_id
    let chatroom_id = ch["chatroom"]["id"].as_u64()
        .or_else(|| ch["chatroom_id"].as_u64())
        .unwrap_or(channel_id);
    let slug = ch["slug"].as_str().unwrap_or(channel).to_string();

    Some(ChannelInfo { channel_id, chatroom_id, slug })
}

async fn try_v1_api(http: &reqwest::Client, channel: &str, token: &str) -> Option<ChannelInfo> {
    let url = format!("https://kick.com/api/v1/channels/{channel}");
    let mut req = http.get(&url);
    if !token.is_empty() {
        req = req.header("Authorization", format!("Bearer {token}"));
    }
    let resp = req.send().await.ok()?;

    let status = resp.status();
    if !status.is_success() {
        let body = resp.text().await.unwrap_or_default();
        warn!("Kick API v1 {channel}: {status} — {body}");
        return None;
    }

    let json: serde_json::Value = resp.json().await.ok()?;

    let channel_id  = json["user_id"].as_u64().or_else(|| json["id"].as_u64())?;
    let chatroom_id = json["chatroom"]["id"].as_u64()?;
    let slug        = json["slug"].as_str().unwrap_or(channel).to_string();

    Some(ChannelInfo { channel_id, chatroom_id, slug })
}
