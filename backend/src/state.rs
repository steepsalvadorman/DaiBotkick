use crate::{cooldown::CooldownManager, queue::VideoQueue, tts};
use dashmap::DashMap;
use std::sync::{atomic::AtomicU64, Arc};
use tokio::sync::{mpsc, Mutex, RwLock};

pub struct SorteoState {
    pub open:         bool,
    pub participants: Vec<String>,
}

pub struct ChannelCommands {
    pub discord: String,
    pub redes:   String,
    pub pc:      String,
    pub horario: String,
}

pub struct GlobalConfig {
    pub client_id:     String,
    pub client_secret: String,
    pub port:          u16,
    pub overlay_dir:   String,
    pub base_url:      String,
    pub tts_cache_dir: String,
}

/// Estado de un canal específico (un streamer).
pub struct ChannelState {
    pub slug:              String,
    pub access_token:      Arc<RwLock<String>>,
    pub refresh_token_val: Arc<RwLock<String>>,
    pub channel_id:        Arc<RwLock<Option<u64>>>,
    pub chatroom_id:       Arc<RwLock<Option<u64>>>,
    pub followers:         Arc<AtomicU64>,
    pub follow_goal:       u64,
    pub video_queue:       Arc<RwLock<VideoQueue>>,
    pub tts_tx:            mpsc::UnboundedSender<tts::TtsQueueItem>,
    pub last_advance:      Arc<Mutex<Option<std::time::Instant>>>,
    pub start_time:        std::time::Instant,
    pub sorteo:            Arc<Mutex<SorteoState>>,
    pub cooldown:          Arc<Mutex<CooldownManager>>,
    pub commands:          ChannelCommands,
    pub panel_token:       String,
}

/// Estado global del servidor (compartido entre todos los canales).
pub struct AppState {
    pub config:          GlobalConfig,
    pub http:            reqwest::Client,
    pub io:              socketioxide::SocketIo,
    pub db:              sqlx::PgPool,
    /// slug → ChannelState
    pub channels:        Arc<DashMap<String, Arc<ChannelState>>>,
    /// broadcaster_user_id → slug (para rutear webhooks)
    pub user_id_to_slug: Arc<DashMap<u64, String>>,
}
