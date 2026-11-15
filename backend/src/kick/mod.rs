pub mod ingest;
pub mod sender;
use crate::state::{AppState, ChannelState};
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

pub use crate::oauth::{refresh_access_token, refresh_if_current};

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
            let detail: String = r
                .text()
                .await
                .unwrap_or_default()
                .chars()
                .take(300)
                .collect();
            tracing::warn!("[EventSub][{}] HTTP {} {}", ch.slug, status, detail);
            false
        }
        Err(_) => {
            tracing::warn!("[EventSub][{}] Error de red", ch.slug);
            false
        }
    }
}
