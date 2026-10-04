use crate::{
    cooldown::CooldownManager,
    db::ChannelRow,
    kick,
    queue::VideoQueue,
    state::{AppState, ChannelCommands, ChannelState, SorteoState},
    stats, tts,
};
use std::sync::{
    atomic::{AtomicBool, AtomicU64},
    Arc,
};
use tokio::sync::{mpsc, Mutex, RwLock};

pub async fn start_channel(row: ChannelRow, global: Arc<AppState>) -> Result<(), String> {
    let queue: VideoQueue =
        serde_json::from_str(&row.queue_state).map_err(|_| "Cola persistida invalida")?;
    let slug = row.slug.clone();
    let _guard = global.channel_lock.lock().await;
    if let Some((_, old)) = global.channels.remove(&slug) {
        old.cancel.cancel();
        global.user_id_to_slug.retain(|_, v| v != &slug);
    }
    let (tts_tx, tts_rx) = mpsc::channel(32);
    let ch = Arc::new(ChannelState {
        slug: slug.clone(),
        access_token: Arc::new(RwLock::new(row.access_token)),
        persisted_refresh_token: RwLock::new(row.refresh_token.clone()),
        refresh_token_val: Arc::new(RwLock::new(row.refresh_token)),
        channel_id: Arc::new(RwLock::new(row.broadcaster_user_id.map(|v| v as u64))),
        followers: Arc::new(AtomicU64::new(0)),
        followers_known: AtomicBool::new(false),
        follow_goal: AtomicU64::new(row.follow_goal.max(0) as u64),
        video_queue: Arc::new(RwLock::new(queue)),
        tts_tx,
        bingo: Mutex::new(crate::bingo::Bingo::default()),
        sorteo: Arc::new(Mutex::new(SorteoState {
            open: false,
            participants: Vec::new(),
        })),
        cooldown: Arc::new(Mutex::new(CooldownManager::new())),
        commands: RwLock::new(ChannelCommands {
            discord: row.cmd_discord,
            redes: row.cmd_redes,
            pc: row.cmd_pc,
            horario: row.cmd_horario,
        }),
        panel_token: row.panel_token,
        playback_token: row.playback_token,
        cancel: global.shutdown.child_token(),
        refresh_lock: Mutex::new(()),
        token_expires: AtomicU64::new(row.token_expires.max(0) as u64),
        player: Mutex::new(None),
        show_video: AtomicBool::new(row.show_video),
        live_since: RwLock::new(None),
        recent_sent: Mutex::new(std::collections::VecDeque::new()),
    });
    if let Some(id) = row.broadcaster_user_id {
        global.user_id_to_slug.insert(id as u64, slug.clone());
    }
    global.channels.insert(slug.clone(), ch.clone());
    // Reconnect existing sockets so they acquire the new state and authorization.
    global.io.to(slug.clone()).disconnect().ok();
    let service = Arc::new(tts::TtsService::new(
        &global.config.tts_cache_dir,
        &global.config.fish_audio_api_key,
    ));
    let (io, cancel, slots) = (
        global.io.clone(),
        ch.cancel.clone(),
        global.tts_slots.clone(),
    );
    let tts_slug = slug.clone();
    tokio::spawn(async move {
        tokio::select! { _ = cancel.cancelled() => {}, _ = tts::spawn_processor(service, tts_rx, io, tts_slug, slots) => {} }
    });
    let (worker, app) = (ch.clone(), global.clone());
    tokio::spawn(async move {
        tokio::select! { _ = worker.cancel.cancelled() => {}, _ = kick::run_channel(worker.clone(), app) => {} }
    });
    stats::start_channel(global.io.clone(), ch, global.clone());
    tracing::info!("[Channel] Iniciado: {slug}");
    Ok(())
}
