//! Renovación de tokens OAuth de los canales, común a Kick y Twitch: ambos usan
//! `grant_type=refresh_token` y pueden rotar el refresh token en cada renovación.

use crate::{
    db,
    platform::Platform,
    state::{AppState, ChannelState},
};
use std::sync::{atomic::Ordering, Arc};

/// URL de tokens y credenciales de la app de la plataforma; `None` si Twitch no está configurado.
pub fn token_endpoint(platform: Platform, global: &AppState) -> Option<(String, &str, &str)> {
    match platform {
        Platform::Kick => Some((
            format!("{}/token", global.kick_endpoints.oauth),
            &global.config.client_id,
            &global.config.client_secret,
        )),
        Platform::Twitch => global.config.twitch.as_ref().map(|tw| {
            (
                format!("{}/token", global.twitch_endpoints.oauth),
                tw.client_id.as_str(),
                tw.client_secret.as_str(),
            )
        }),
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
    let Some((token_url, client_id, client_secret)) = token_endpoint(ch.platform, global) else {
        return false;
    };
    let response = global
        .http
        .post(token_url)
        .form(&[
            ("grant_type", "refresh_token"),
            ("refresh_token", refresh.as_str()),
            ("client_id", client_id),
            ("client_secret", client_secret),
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
        ch.platform,
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
            if let Ok(row) = db::load_channel(&global.db, ch.platform, &ch.slug).await {
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
