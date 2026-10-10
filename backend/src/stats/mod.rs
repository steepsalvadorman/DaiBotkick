use crate::state::{AppState, ChannelState};
use serde_json::json;
use std::sync::{atomic::Ordering, Arc};

pub fn start_channel(io: socketioxide::SocketIo, ch: Arc<ChannelState>, global: Arc<AppState>) {
    tokio::spawn(async move {
        let mut ticker = tokio::time::interval(std::time::Duration::from_secs(60));
        loop {
            tokio::select! { _ = ch.cancel.cancelled() => return, _ = ticker.tick() => {} }
            tokio::select! { _ = ch.cancel.cancelled() => return, _ = fetch(&global, &ch, &io) => {} }
        }
    });
}
async fn fetch(global: &AppState, ch: &ChannelState, io: &socketioxide::SocketIo) {
    let token = ch.access_token.read().await.clone();
    let Ok(response) = global
        .http
        .get(format!("{}/channels", global.kick_endpoints.api))
        .query(&[(
            "broadcaster_user_id",
            ch.channel_id.read().await.unwrap_or(0).to_string(),
        )])
        .bearer_auth(token)
        .send()
        .await
    else {
        return;
    };
    if !response.status().is_success() {
        tracing::warn!("[Stats][{}] HTTP {}", ch.slug, response.status());
        return;
    }
    let Ok(data) = response.json::<serde_json::Value>().await else {
        return;
    };
    let Some(channel) = data["data"].as_array().and_then(|a| a.first()) else {
        return;
    };
    let stream = &channel["stream"];
    let live = stream["is_live"].as_bool().unwrap_or(false);
    let viewers = stream["viewer_count"]
        .as_u64()
        .or_else(|| channel["viewer_count"].as_u64());
    io.to(ch.slug.clone())
        .emit("viewerCount", json!({"count":viewers}))
        .ok();
    let started = if live {
        stream["start_time"]
            .as_str()
            .or_else(|| stream["started_at"].as_str())
            .and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok())
            .map(|t| t.with_timezone(&chrono::Utc))
    } else {
        None
    };
    *ch.live_since.write().await = started;
    io.to(ch.slug.clone())
        .emit(
            "streamStatus",
            json!({"live":live,"startedAt":started.map(|t| t.to_rfc3339())}),
        )
        .ok();
    let mut followers = channel["followers_count"]
        .as_u64()
        .or_else(|| channel["follower_count"].as_u64());
    if followers.is_none() {
        // La API pública oficial no incluye seguidores; la web de Kick sí.
        followers = legacy_followers(global, &ch.slug).await;
    }
    if let Some(count) = followers {
        ch.followers.store(count, Ordering::Relaxed);
    }
    ch.followers_known
        .store(followers.is_some(), Ordering::Relaxed);
    io.to(ch.slug.clone())
        .emit("followersUpdate", json!({"count":followers}))
        .ok();
}

async fn legacy_followers(global: &AppState, slug: &str) -> Option<u64> {
    let response = global
        .http
        .get(format!("https://kick.com/api/v2/channels/{slug}"))
        .header(
            reqwest::header::USER_AGENT,
            "Mozilla/5.0 (compatible; CuchurruminRix)",
        )
        .header(reqwest::header::ACCEPT, "application/json")
        .timeout(std::time::Duration::from_secs(10))
        .send()
        .await
        .ok()?;
    if !response.status().is_success() {
        tracing::debug!(
            "[Stats][{slug}] seguidores (web) HTTP {}",
            response.status()
        );
        return None;
    }
    let json: serde_json::Value = response.json().await.ok()?;
    json["followers_count"]
        .as_u64()
        .or_else(|| json["followersCount"].as_u64())
}
