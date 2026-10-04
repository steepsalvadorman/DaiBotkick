use crate::{cooldown::CooldownManager, queue::VideoQueue, tts};
use dashmap::DashMap;
use std::sync::{atomic::AtomicU64, Arc};
use tokio::sync::{mpsc, Mutex, RwLock, Semaphore};
use tokio_util::sync::CancellationToken;

pub struct SorteoState {
    pub open: bool,
    pub participants: Vec<String>,
}

pub struct ChannelCommands {
    pub discord: String,
    pub redes: String,
    pub pc: String,
    pub horario: String,
}

pub struct GlobalConfig {
    pub client_id: String,
    pub client_secret: String,
    pub port: u16,
    pub overlay_dir: String,
    pub base_url: String,
    pub tts_cache_dir: String,
}

/// Estado de un canal específico (un streamer).
pub struct ChannelState {
    pub slug: String,
    pub access_token: Arc<RwLock<String>>,
    pub refresh_token_val: Arc<RwLock<String>>,
    /// Last DB value, retained across failed writes to recover rotated tokens safely.
    pub persisted_refresh_token: RwLock<String>,
    pub channel_id: Arc<RwLock<Option<u64>>>,
    pub followers: Arc<AtomicU64>,
    pub followers_known: std::sync::atomic::AtomicBool,
    pub follow_goal: AtomicU64,
    pub video_queue: Arc<RwLock<VideoQueue>>,
    pub tts_tx: mpsc::Sender<tts::TtsQueueItem>,
    pub sorteo: Arc<Mutex<SorteoState>>,
    pub bingo: Mutex<crate::bingo::Bingo>,
    pub cooldown: Arc<Mutex<CooldownManager>>,
    /// Textos de !discord, !redes, !pc y !horario; el streamer los cambia con !set.
    pub commands: RwLock<ChannelCommands>,
    pub panel_token: String,
    pub playback_token: String,
    pub cancel: CancellationToken,
    pub refresh_lock: Mutex<()>,
    pub token_expires: AtomicU64,
    pub player: Mutex<Option<String>>,
    pub show_video: std::sync::atomic::AtomicBool,
    pub live_since: RwLock<Option<chrono::DateTime<chrono::Utc>>>,
    /// Últimos mensajes que envió el bot; su eco en el webhook no se muestra en el overlay.
    pub recent_sent: Mutex<std::collections::VecDeque<String>>,
}

/// Estado global del servidor (compartido entre todos los canales).
pub struct AppState {
    pub config: GlobalConfig,
    pub http: reqwest::Client,
    pub kick_endpoints: crate::kick::Endpoints,
    pub io: socketioxide::SocketIo,
    pub db: sqlx::PgPool,
    /// slug → ChannelState
    pub channels: Arc<DashMap<String, Arc<ChannelState>>>,
    /// broadcaster_user_id → slug (para rutear webhooks)
    pub user_id_to_slug: Arc<DashMap<u64, String>>,
    pub channel_lock: Mutex<()>,
    pub shutdown: CancellationToken,
    pub tts_slots: Arc<Semaphore>,
    pub search_slots: Semaphore,
    pub webhook_key: rsa::RsaPublicKey,
    pub metrics: Arc<Metrics>,
}

#[derive(Default)]
pub struct Metrics {
    pub webhook_rejected: AtomicU64,
    pub webhook_received: AtomicU64,
    pub webhook_duplicates: AtomicU64,
    pub webhook_processed: AtomicU64,
}
