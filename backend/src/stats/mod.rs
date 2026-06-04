use crate::state::{AppState, ChannelState};
use serde_json::json;
use std::sync::{atomic::Ordering, Arc};
use sysinfo::{Components, CpuRefreshKind, MemoryRefreshKind, RefreshKind, System};
use tokio::time::{interval, Duration};
use tracing::debug;

pub fn start_channel(io: socketioxide::SocketIo, ch: Arc<ChannelState>, global: Arc<AppState>) {
    let slug = ch.slug.clone();
    tokio::spawn(async move {
        let mut sys = System::new_with_specifics(
            RefreshKind::new()
                .with_cpu(CpuRefreshKind::everything())
                .with_memory(MemoryRefreshKind::everything()),
        );
        let mut components = Components::new_with_refreshed_list();
        sys.refresh_cpu_all();
        tokio::time::sleep(Duration::from_millis(250)).await;

        let mut ticker    = interval(Duration::from_secs(2));
        let mut poll_tick = 0u8;

        loop {
            ticker.tick().await;
            poll_tick = poll_tick.wrapping_add(1);

            sys.refresh_specifics(
                RefreshKind::new()
                    .with_cpu(CpuRefreshKind::everything())
                    .with_memory(MemoryRefreshKind::everything()),
            );
            components.refresh();

            // ── Stats del sistema (no se muestran en overlay pero pueden usarse) ──
            let _cpu = sys.global_cpu_usage();
            let _ram = {
                let t = sys.total_memory();
                let u = sys.used_memory();
                if t > 0 { (u as f64 / t as f64) * 100.0 } else { 0.0 }
            };
            let _cpu_temp = components.iter()
                .filter(|c| {
                    let l = c.label().to_lowercase();
                    l.contains("package") || l == "cpu" || l.contains("tdie")
                })
                .map(|c| c.temperature())
                .filter(|t| t.is_finite() && *t > 0.0)
                .next()
                .or_else(|| {
                    components.iter()
                        .filter(|c| c.label().to_lowercase().contains("core"))
                        .map(|c| c.temperature())
                        .filter(|t| t.is_finite() && *t > 0.0)
                        .next()
                })
                .or_else(read_thermal_zone);

            // ── Meta de seguidores (cada 2s) ─────────────────────────────────────
            let followers   = ch.followers.load(Ordering::Relaxed);
            let follow_goal = ch.follow_goal;
            io.to(slug.clone()).emit("followGoal", json!({
                "current": followers,
                "goal":    follow_goal,
            })).ok();

            // ── Viewer count (cada 60s via API de Kick) ───────────────────────────
            if poll_tick % 30 == 0 {
                let token = ch.access_token.read().await.clone();
                fetch_viewers(&global, &slug, &token, &io).await;
            }
        }
    });
}

async fn fetch_viewers(
    global: &Arc<AppState>,
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

    let viewers = json["data"][0]["viewers_count"].as_u64()
        .or_else(|| json["data"][0]["viewer_count"].as_u64())
        .unwrap_or(0);

    debug!("[Stats][{slug}] Viewers: {viewers}");
    io.to(slug.to_owned()).emit("viewerCount", json!({ "count": viewers })).ok();
}

#[cfg(target_os = "linux")]
fn read_thermal_zone() -> Option<f32> {
    std::fs::read_to_string("/sys/class/thermal/thermal_zone0/temp")
        .ok()
        .and_then(|s| s.trim().parse::<f32>().ok())
        .map(|t| t / 1000.0)
        .filter(|t| *t > 0.0 && t.is_finite())
}

#[cfg(not(target_os = "linux"))]
fn read_thermal_zone() -> Option<f32> { None }
