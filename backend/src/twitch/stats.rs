//! Espectadores, seguidores y estado del directo de un canal de Twitch, cada 60 s, para el
//! overlay, `!seguidores` y `!uptime` (que también se actualiza al instante por EventSub).

use crate::{
    server::ns_emit,
    state::{AppState, ChannelState},
};
use serde_json::json;
use std::sync::{atomic::Ordering, Arc};

pub async fn run(ch: Arc<ChannelState>, app: Arc<AppState>) {
    let mut ticker = tokio::time::interval(std::time::Duration::from_secs(60));
    loop {
        ticker.tick().await;
        let Some(id) = *ch.channel_id.read().await else {
            continue;
        };
        refresh(&ch, &app, id).await;
    }
}

pub(crate) async fn refresh(ch: &Arc<ChannelState>, app: &Arc<AppState>, id: u64) {
    let stream = super::helix(ch, app, |http, api| {
        http.get(format!("{api}/streams"))
            .query(&[("user_id", id.to_string())])
    })
    .await;
    if let Ok(json) = read(stream).await {
        let (live, viewers, started) = stream_status(&json["data"][0]);
        ns_emit(app, ch, "viewerCount", json!({"count": viewers}));
        crate::chat::live_status(app, ch, started, live).await;
    }
    let followers = super::helix(ch, app, |http, api| {
        http.get(format!("{api}/channels/followers"))
            .query(&[("broadcaster_id", id.to_string()), ("first", "1".into())])
    })
    .await;
    let total = read(followers).await.ok().and_then(|j| j["total"].as_u64());
    if let Some(count) = total {
        ch.followers.store(count, Ordering::Relaxed);
    }
    ch.followers_known.store(total.is_some(), Ordering::Relaxed);
    ns_emit(app, ch, "followersUpdate", json!({"count": total}));
}

async fn read(
    response: Result<reqwest::Response, super::HelixError>,
) -> Result<serde_json::Value, ()> {
    response.map_err(|_| ())?.json().await.map_err(|_| ())
}

/// `GET /helix/streams` solo devuelve el directo si está en vivo.
fn stream_status(
    stream: &serde_json::Value,
) -> (bool, Option<u64>, Option<chrono::DateTime<chrono::Utc>>) {
    let live = stream["type"] == "live";
    (
        live,
        live.then(|| stream["viewer_count"].as_u64()).flatten(),
        crate::chat::parse_time(&stream["started_at"]).filter(|_| live),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn offline_channels_have_no_viewers_or_start_time() {
        let (live, viewers, started) = stream_status(&json!({
            "type": "live", "viewer_count": 42, "started_at": "2026-11-15T18:00:00Z"
        }));
        assert!(live);
        assert_eq!(viewers, Some(42));
        assert_eq!(started.unwrap().to_rfc3339(), "2026-11-15T18:00:00+00:00");
        assert_eq!(stream_status(&serde_json::Value::Null), (false, None, None));
    }
}
