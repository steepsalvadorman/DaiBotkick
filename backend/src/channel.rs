use crate::{
    cooldown::CooldownManager,
    db::ChannelRow,
    kick, stats, tts,
    state::{AppState, ChannelCommands, ChannelState, SorteoState},
    queue::VideoQueue,
};
use std::sync::{atomic::AtomicU64, Arc};
use tokio::sync::{mpsc, Mutex, RwLock};
use tracing::info;

fn unix_now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

/// Inicializa un canal: crea su ChannelState, registra el namespace Socket.IO,
/// y arranca los tasks de Kick, TTS y stats.
pub async fn start_channel(row: ChannelRow, global: Arc<AppState>) {
    let slug = row.slug.clone();

    // Si ya está corriendo, detener primero (reconexión tras re-login)
    global.channels.remove(&slug);

    let (tts_tx, tts_rx) = mpsc::unbounded_channel::<tts::TtsQueueItem>();

    let token_expires = if row.token_expires > 0 { row.token_expires as u64 } else { unix_now() + 7200 };

    let ch = Arc::new(ChannelState {
        slug:              slug.clone(),
        access_token:      Arc::new(RwLock::new(row.access_token.clone())),
        refresh_token_val: Arc::new(RwLock::new(row.refresh_token.clone())),
        channel_id:        Arc::new(RwLock::new(row.broadcaster_user_id.map(|v| v as u64))),
        chatroom_id:       Arc::new(RwLock::new(row.chatroom_id.map(|v| v as u64))),
        followers:         Arc::new(AtomicU64::new(0)),
        follow_goal:       row.follow_goal as u64,
        video_queue:       Arc::new(RwLock::new(VideoQueue::new())),
        tts_tx,
        start_time:        std::time::Instant::now(),
        sorteo:            Arc::new(Mutex::new(SorteoState { open: false, participants: Vec::new() })),
        cooldown:          Arc::new(Mutex::new(CooldownManager::new())),
        commands:          ChannelCommands {
            discord: row.cmd_discord,
            redes:   row.cmd_redes,
            pc:      row.cmd_pc,
            horario: row.cmd_horario,
        },
        panel_token: row.panel_token,
    });

    // Registrar broadcaster_id → slug para ruteo de webhooks
    if let Some(bid) = row.broadcaster_user_id {
        global.user_id_to_slug.insert(bid as u64, slug.clone());
    }

    // Guardar en el mapa global
    global.channels.insert(slug.clone(), ch.clone());

    // TTS processor para este canal
    let tts_service = Arc::new(tts::TtsService::new(&global.config.tts_cache_dir));
    let tts_io  = global.io.clone();
    let tts_slug = slug.clone();
    tokio::spawn(async move {
        tts::spawn_processor(tts_service, tts_rx, tts_io, tts_slug).await;
    });

    // Kick: WebSocket + EventSub
    let ch2      = ch.clone();
    let global2  = global.clone();
    tokio::spawn(async move {
        kick::run_channel(ch2, global2, token_expires).await;
    });

    // Stats (CPU/RAM/followGoal por canal)
    stats::start_channel(global.io.clone(), ch.clone(), global.clone());

    info!("[Channel] Iniciado: {slug}");
}
