pub mod sender;
use crate::{
    db,
    state::{AppState, ChannelState},
};
use std::sync::{atomic::Ordering, Arc};
use tokio::time::{sleep, Duration};

/// Production defaults; integration tests inject a local HTTP service.
pub struct Endpoints {
    pub oauth: String,
    pub api: String,
}

impl Default for Endpoints {
    fn default() -> Self {
        Self {
            oauth: "https://id.kick.com/oauth".into(),
            api: "https://api.kick.com/public/v1".into(),
        }
    }
}

pub async fn refresh_access_token(ch: &Arc<ChannelState>, global: &Arc<AppState>) -> bool {
    let observed = ch.access_token.read().await.clone();
    refresh_if_current(ch, global, &observed).await
}

pub async fn refresh_if_current(
    ch: &Arc<ChannelState>,
    global: &Arc<AppState>,
    observed: &str,
) -> bool {
    if ch.cancel.is_cancelled() {
        return false;
    }
    let _guard = ch.refresh_lock.lock().await;
    if ch.cancel.is_cancelled() {
        return false;
    }
    if *ch.access_token.read().await != observed {
        return true;
    }
    let refresh = ch.refresh_token_val.read().await.clone();
    let persisted_refresh = ch.persisted_refresh_token.read().await.clone();
    if refresh.is_empty() {
        return false;
    }
    let response = global
        .http
        .post(format!("{}/token", global.kick_endpoints.oauth))
        .form(&[
            ("grant_type", "refresh_token"),
            ("refresh_token", refresh.as_str()),
            ("client_id", global.config.client_id.as_str()),
            ("client_secret", global.config.client_secret.as_str()),
        ])
        .send()
        .await;
    let Ok(response) = response else {
        return false;
    };
    if !response.status().is_success() {
        tracing::warn!(
            "[OAuth][{}] Refresh rechazado: {}",
            ch.slug,
            response.status()
        );
        return false;
    }
    let Ok(data) = response.json::<serde_json::Value>().await else {
        return false;
    };
    let Some(access) = data["access_token"].as_str().filter(|s| !s.is_empty()) else {
        return false;
    };
    let next_refresh = data["refresh_token"].as_str().unwrap_or(&refresh);
    let expires =
        chrono::Utc::now().timestamp().max(0) as u64 + data["expires_in"].as_u64().unwrap_or(7200);
    // Keep rotated tokens in memory even if persistence fails; otherwise the old refresh token is unusable.
    *ch.access_token.write().await = access.to_owned();
    *ch.refresh_token_val.write().await = next_refresh.to_owned();
    ch.token_expires.store(expires, Ordering::Relaxed);
    match db::update_tokens(
        &global.db,
        &ch.slug,
        access,
        next_refresh,
        expires as i64,
        &persisted_refresh,
    )
    .await
    {
        Ok(true) => {
            *ch.persisted_refresh_token.write().await = next_refresh.to_owned();
        }
        Ok(false) => {
            if let Ok(row) = db::load_channel(&global.db, &ch.slug).await {
                *ch.access_token.write().await = row.access_token;
                *ch.persisted_refresh_token.write().await = row.refresh_token.clone();
                *ch.refresh_token_val.write().await = row.refresh_token;
                ch.token_expires
                    .store(row.token_expires.max(0) as u64, Ordering::Relaxed);
            }
            return false;
        }
        Err(e) => {
            tracing::error!("[OAuth][{}] No se pudieron persistir tokens: {e}", ch.slug);
            return false;
        }
    }
    true
}

pub async fn run_channel(ch: Arc<ChannelState>, global: Arc<AppState>) {
    let subscriptions = async {
        sleep(Duration::from_secs(5)).await;
        let mut delay = 5;
        loop {
            if subscribe(&ch, &global).await {
                break;
            }
            sleep(Duration::from_secs(delay)).await;
            delay = (delay * 2).min(300);
        }
    };
    let refresh = async {
        loop {
            let now = chrono::Utc::now().timestamp().max(0) as u64;
            let remaining = ch.token_expires.load(Ordering::Relaxed).saturating_sub(now);
            sleep(Duration::from_secs(remaining.saturating_sub(600).max(5))).await;
            if !refresh_access_token(&ch, &global).await {
                sleep(Duration::from_secs(60)).await;
            }
        }
    };
    tokio::join!(subscriptions, refresh);
}

async fn subscribe(ch: &Arc<ChannelState>, global: &Arc<AppState>) -> bool {
    let Some(id) = *ch.channel_id.read().await else {
        return false;
    };
    let token = ch.access_token.read().await.clone();
    let body = serde_json::json!({"events": [
        {"name":"chat.message.sent","version":1}, {"name":"channel.followed","version":1},
        {"name":"channel.subscription.new","version":1}, {"name":"channel.subscription.renewal","version":1},
        {"name":"channel.subscription.gifts","version":1}, {"name":"livestream.status.updated","version":1}
    ], "method":"webhook", "broadcaster_user_id":id});
    match global
        .http
        .post(format!(
            "{}/events/subscriptions",
            global.kick_endpoints.api
        ))
        .bearer_auth(token)
        .json(&body)
        .send()
        .await
    {
        Ok(r) if r.status().is_success() || r.status() == reqwest::StatusCode::CONFLICT => {
            tracing::info!("[EventSub][{}] Suscripciones listas", ch.slug);
            true
        }
        Ok(r) => {
            let status = r.status();
            let detail: String = r.text().await.unwrap_or_default().chars().take(300).collect();
            tracing::warn!("[EventSub][{}] HTTP {} {}", ch.slug, status, detail);
            false
        }
        Err(_) => {
            tracing::warn!("[EventSub][{}] Error de red", ch.slug);
            false
        }
    }
}
