use crate::state::{AppState, ChannelState};
use serde_json::json;
use std::sync::{atomic::Ordering, Arc};
use sysinfo::{CpuRefreshKind, MemoryRefreshKind, RefreshKind, System};
use tokio::time::{interval, Duration};

pub fn start_channel(io: socketioxide::SocketIo, ch: Arc<ChannelState>, global: Arc<AppState>) {
    let slug = ch.slug.clone();
    tokio::spawn(async move {
        let mut sys = System::new_with_specifics(
            RefreshKind::new()
                .with_cpu(CpuRefreshKind::everything())
                .with_memory(MemoryRefreshKind::everything()),
        );
        sys.refresh_cpu_all();
        tokio::time::sleep(Duration::from_millis(250)).await;

        let mut ticker    = interval(Duration::from_secs(2));
        let mut poll_tick = 0u8;

        // Fetch inicial inmediato
        let token = ch.access_token.read().await.clone();
        fetch_channel_stats(&global, &ch, &slug, &token, &io).await;

        loop {
            ticker.tick().await;
            poll_tick = poll_tick.wrapping_add(1);

            // Cada 60s: viewers + followers reales de la API
            if poll_tick % 30 == 0 {
                let token = ch.access_token.read().await.clone();
                fetch_channel_stats(&global, &ch, &slug, &token, &io).await;
            }
        }
    });
}

async fn fetch_channel_stats(
    global: &Arc<AppState>,
    ch:     &Arc<ChannelState>,
    slug:   &str,
    token:  &str,
    io:     &socketioxide::SocketIo,
) {
    if token.is_empty() { return; }

    let url = format!(
        "https://api.kick.com/public/v1/channels?broadcaster_username={slug}"
    );
    let Ok(resp) = global.http
        .get(&url)
        .header("Authorization", format!("Bearer {token}"))
        .send()
        .await
    else { return };

    let Ok(json) = resp.json::<serde_json::Value>().await else { return };
    let ch_data = &json["data"][0];

    // Log temporal para ver qué campos devuelve la API
    tracing::info!("[Stats][{slug}] API raw: {}", ch_data);

    // Viewers (puede estar anidado bajo "stream")
    let viewers = ch_data["viewers_count"].as_u64()
        .or_else(|| ch_data["viewer_count"].as_u64())
        .or_else(|| ch_data["stream"]["viewers_count"].as_u64())
        .or_else(|| ch_data["stream"]["viewer_count"].as_u64())
        .unwrap_or(0);
    tracing::info!("[Stats][{slug}] Viewers: {viewers}");
    io.to(slug.to_owned()).emit("viewerCount", json!({ "count": viewers })).ok();

    // Followers — probamos todos los nombres posibles
    let followers = ch_data["followers_count"].as_u64()
        .or_else(|| ch_data["follower_count"].as_u64())
        .or_else(|| ch_data["followers"].as_u64())
        .or_else(|| ch_data["subscriber_count"].as_u64());
    if let Some(f) = followers {
        tracing::info!("[Stats][{slug}] Followers: {f}");
        ch.followers.store(f, Ordering::Relaxed);
        io.to(slug.to_owned()).emit("followersUpdate", json!({ "count": f })).ok();
    } else {
        tracing::info!("[Stats][{slug}] Followers: campo no encontrado en API");
    }
}

#[cfg(target_os = "linux")]
fn _read_thermal_zone() -> Option<f32> {
    std::fs::read_to_string("/sys/class/thermal/thermal_zone0/temp")
        .ok()
        .and_then(|s| s.trim().parse::<f32>().ok())
        .map(|t| t / 1000.0)
        .filter(|t| *t > 0.0 && t.is_finite())
}
