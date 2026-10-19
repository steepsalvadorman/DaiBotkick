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
    pub fish_audio_api_key: String,
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
    /// Nombre de usuario tal como lo escribió el streamer en Kick (p. ej. `Frank_Arr`);
    /// vacío hasta consultarlo. El slug (`frank-arr`) solo sirve para URLs.
    pub display_name: RwLock<String>,
    /// Foto de perfil del streamer en Kick (URL https); vacía hasta consultarla.
    pub avatar_url: RwLock<String>,
}

impl ChannelState {
    /// Datos del canal para el overlay: nombre original si se conoce, si no el slug.
    pub async fn overlay_config(&self) -> serde_json::Value {
        let name = self.display_name.read().await.clone();
        let name = if name.is_empty() {
            self.slug.clone()
        } else {
            name
        };
        let avatar = self.avatar_url.read().await.clone();
        let mut config = serde_json::json!({"channel_name": name, "kick_url": format!("kick.com/{}", self.slug)});
        if !avatar.is_empty() {
            config["avatar_url"] = avatar.into();
        }
        config
    }
}

/// Acepta el nombre de Kick solo si corresponde al slug del canal: Kick genera el slug
/// en minúsculas y cambiando `_` por `-`, así que el nombre conserva ambos tal cual.
pub fn avatar_url_valid(url: &str) -> bool {
    url.len() <= 500
        && url.strip_prefix("https://").is_some_and(|rest| {
            !rest.is_empty()
                && !rest
                    .chars()
                    .any(|c| c.is_whitespace() || matches!(c, '"' | '\'' | '<' | '>'))
        })
}

pub fn display_name_matches(name: &str, slug: &str) -> bool {
    !name.is_empty()
        && name.len() <= 100
        && name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
        && name.to_ascii_lowercase().replace('_', "-")
            == slug.to_ascii_lowercase().replace('_', "-")
}

#[cfg(test)]
mod tests {
    use super::{avatar_url_valid, display_name_matches};

    #[test]
    fn avatar_url_only_accepts_plain_https_urls() {
        assert!(avatar_url_valid(
            "https://files.kick.com/images/user/1/profile_image/a.webp"
        ));
        assert!(!avatar_url_valid("http://files.kick.com/a.webp"));
        assert!(!avatar_url_valid("https://"));
        assert!(!avatar_url_valid("https://x.com/a\" onerror=\"x"));
        assert!(!avatar_url_valid("javascript:alert(1)"));
        assert!(!avatar_url_valid(""));
    }

    #[test]
    fn display_name_keeps_underscores_and_case_only_for_the_same_channel() {
        assert!(display_name_matches("Frank_Arr", "frank-arr"));
        assert!(display_name_matches("Frank_Arr", "frank_arr"));
        assert!(display_name_matches("Senior-Dai", "senior-dai"));
        assert!(display_name_matches("SeniorDai", "seniordai"));
        assert!(!display_name_matches("OtroCanal", "frank-arr"));
        assert!(!display_name_matches("Frank_Arr2", "frank-arr"));
        assert!(!display_name_matches("<b>Frank_Arr</b>", "frank-arr"));
        assert!(!display_name_matches("", "frank-arr"));
    }
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
    pub bingo_auth: crate::bingo_auth::ViewerAuth,
}

#[derive(Default)]
pub struct Metrics {
    pub webhook_rejected: AtomicU64,
    pub webhook_received: AtomicU64,
    pub webhook_duplicates: AtomicU64,
    pub webhook_processed: AtomicU64,
}
