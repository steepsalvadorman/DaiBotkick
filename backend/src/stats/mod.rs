use crate::state::ChannelState;
use std::sync::{atomic::Ordering, Arc};
use sysinfo::{CpuRefreshKind, MemoryRefreshKind, RefreshKind, System};
use tokio::time::{interval, Duration};

pub fn start_channel(io: socketioxide::SocketIo, ch: Arc<ChannelState>) {
    let slug = ch.slug.clone();
    tokio::spawn(async move {
        let mut sys = System::new_with_specifics(
            RefreshKind::new()
                .with_cpu(CpuRefreshKind::everything())
                .with_memory(MemoryRefreshKind::everything()),
        );
        sys.refresh_cpu_all();
        tokio::time::sleep(Duration::from_millis(250)).await;
        let mut ticker = interval(Duration::from_secs(2));
        loop {
            ticker.tick().await;
            sys.refresh_specifics(
                RefreshKind::new()
                    .with_cpu(CpuRefreshKind::everything())
                    .with_memory(MemoryRefreshKind::everything()),
            );
            let cpu = sys.global_cpu_usage();
            let ram = { let t = sys.total_memory(); let u = sys.used_memory();
                if t > 0 { (u as f64 / t as f64) * 100.0 } else { 0.0 } };
            let followers   = ch.followers.load(Ordering::Relaxed);
            let follow_goal = ch.follow_goal;

            io.to(slug.clone()).emit("sysStats", serde_json::json!({
                "cpu": format!("{:.0}", cpu),
                "ram": format!("{:.1}", ram),
            })).ok();
            io.to(slug.clone()).emit("followGoal", serde_json::json!({
                "current": followers,
                "goal":    follow_goal,
            })).ok();
        }
    });
}
