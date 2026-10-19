pub mod edge_tts;
pub mod fish_audio;
use base64::{engine::general_purpose::STANDARD, Engine};
use std::{path::PathBuf, sync::Arc};
use tokio::sync::{mpsc, Semaphore};

/// Máximo de caracteres de un mensaje de chat en Kick: ningún mensaje de voz puede superarlo.
pub const KICK_MESSAGE_MAX: usize = 500;
/// El prefijo «nombre dice: » usa hasta 40 caracteres del nombre y no consume el mensaje.
const AUTHOR_PREFIX_MAX: usize = 40 + " dice: ".len();

pub struct TtsQueueItem {
    pub text: String,
    pub voice: String,
}
pub fn enqueue(ch: &crate::state::ChannelState, text: &str, voice: &str) -> bool {
    enqueue_with_limit(ch, text, voice, KICK_MESSAGE_MAX)
}

pub fn enqueue_chat(
    ch: &crate::state::ChannelState,
    username: &str,
    text: &str,
    voice: &str,
) -> bool {
    if text.trim().is_empty() || text.chars().count() > KICK_MESSAGE_MAX {
        return false;
    }
    let name: String = username
        .chars()
        .take(40)
        .map(|c| if c.is_alphanumeric() { c } else { ' ' })
        .collect();
    let name = name.split_whitespace().collect::<Vec<_>>().join(" ");
    let name = if name.is_empty() {
        "Un participante"
    } else {
        &name
    };
    enqueue_with_limit(
        ch,
        &format!("{name} dice: {}", text.trim()),
        voice,
        KICK_MESSAGE_MAX + AUTHOR_PREFIX_MAX,
    )
}

fn enqueue_with_limit(
    ch: &crate::state::ChannelState,
    text: &str,
    voice: &str,
    limit: usize,
) -> bool {
    if ch.cancel.is_cancelled()
        || text.trim().is_empty()
        || text.chars().count() > limit
        || !is_valid_voice(voice)
    {
        return false;
    }
    if ch
        .tts_tx
        .try_send(TtsQueueItem {
            text: text.trim().into(),
            voice: voice.into(),
        })
        .is_err()
    {
        tracing::warn!("[TTS][{}] Cola llena", ch.slug);
        return false;
    }
    true
}

pub fn is_valid_voice(voice: &str) -> bool {
    edge_tts::is_valid_voice(voice) || voice == fish_audio::VOICE_ALIAS
}

pub fn voice_list(fish_audio_enabled: bool) -> String {
    let mut commands = vec!["!s / !camila".to_string()];
    commands.extend(
        edge_tts::VOICES
            .iter()
            .filter(|(alias, _)| *alias != "camila")
            .map(|(alias, _)| format!("!{alias}")),
    );
    if fish_audio_enabled {
        commands.push(format!("!{}", fish_audio::VOICE_ALIAS));
    }
    format!(
        "Voces disponibles: {}. Usa el comando seguido de tu mensaje, por ejemplo: !s Hola stream. Máximo {KICK_MESSAGE_MAX} caracteres; cooldown compartido de 15 s.",
        commands.join(" · ")
    )
}

fn cache_identity(text: &str, voice: &str) -> String {
    if voice == fish_audio::VOICE_ALIAS {
        fish_audio::cache_identity(text)
    } else {
        format!("{voice}:{text}")
    }
}

pub struct TtsService {
    cache_dir: PathBuf,
    fish_audio_api_key: String,
    http: reqwest::Client,
}
impl TtsService {
    pub fn new(cache_dir: &str, fish_audio_api_key: &str) -> Self {
        Self {
            cache_dir: PathBuf::from(cache_dir),
            fish_audio_api_key: fish_audio_api_key.to_string(),
            http: reqwest::Client::new(),
        }
    }
    pub async fn generate(&self, text: &str, voice: &str) -> Option<Vec<u8>> {
        use sha2::{Digest, Sha256};
        let hash = Sha256::digest(cache_identity(text, voice));
        let path = self.cache_dir.join(format!("{hash:x}.mp3"));
        if let Ok(bytes) = tokio::fs::read(&path).await {
            return Some(bytes);
        }
        tokio::fs::create_dir_all(&self.cache_dir).await.ok()?;
        let result = if voice == fish_audio::VOICE_ALIAS {
            match tokio::time::timeout(
                std::time::Duration::from_secs(30),
                fish_audio::synthesize(&self.http, &self.fish_audio_api_key, text),
            )
            .await
            {
                Ok(result) => result,
                Err(_) => Err("Fish Audio timeout".to_string()),
            }
        } else {
            edge_tts::synthesize(text, voice).await
        };
        match result {
            Ok(bytes) => {
                // Solo se poda al crecer la caché; los aciertos no recorren el directorio.
                let _ = tokio::fs::write(path, &bytes).await;
                self.prune().await;
                Some(bytes)
            }
            Err(e) => {
                tracing::warn!("[TTS] {e}");
                None
            }
        }
    }
    async fn prune(&self) {
        let Ok(mut entries) = tokio::fs::read_dir(&self.cache_dir).await else {
            return;
        };
        let mut retained = Vec::new();
        while let Ok(Some(entry)) = entries.next_entry().await {
            if entry.path().extension().is_none_or(|x| x != "mp3") {
                continue;
            }
            let Ok(meta) = entry.metadata().await else {
                continue;
            };
            if !meta.is_file() {
                continue;
            }
            let modified = meta.modified().unwrap_or(std::time::UNIX_EPOCH);
            if modified.elapsed().unwrap_or_default().as_secs() > 86400 {
                let _ = tokio::fs::remove_file(entry.path()).await;
            } else {
                retained.push((modified, entry.path()));
            }
        }
        retained.sort_by_key(|(modified, _)| *modified);
        let excess = retained.len().saturating_sub(255);
        for (_, path) in retained.into_iter().take(excess) {
            let _ = tokio::fs::remove_file(path).await;
        }
    }
}

pub async fn spawn_processor(
    service: Arc<TtsService>,
    mut rx: mpsc::Receiver<TtsQueueItem>,
    io: socketioxide::SocketIo,
    ch: Arc<crate::state::ChannelState>,
    slots: Arc<Semaphore>,
) {
    while let Some(item) = rx.recv().await {
        // Solo el overlay con rol de reproducción toca el audio: sin él no se sintetiza nada.
        if player_sid(&ch).await.is_none() {
            continue;
        }
        let audio = {
            let Ok(_permit) = slots.acquire().await else {
                return;
            };
            service.generate(&item.text, &item.voice).await
        };
        let (Some(audio), Some(sid)) = (audio, player_sid(&ch).await) else {
            continue;
        };
        // Directo al socket reproductor, sin difundir a toda la sala: socketioxide no une
        // cada socket a una sala con su propio id. Va en base64 y no como adjunto binario
        // porque engineioxide 0.14 junta evento y adjunto sin separador en long-polling.
        if let Some(socket) = io.get_socket(sid) {
            socket
                .emit(
                    "speak",
                    serde_json::json!({"audioBase64": STANDARD.encode(audio)}),
                )
                .ok();
        }
    }
}

async fn player_sid(ch: &crate::state::ChannelState) -> Option<socketioxide::socket::Sid> {
    ch.player.lock().await.as_deref()?.parse().ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn voice_list_matches_registered_aliases_and_fish_configuration() {
        let without_fish = voice_list(false);
        for &(alias, _) in edge_tts::VOICES {
            assert!(without_fish.contains(&format!("!{alias}")));
        }
        assert!(without_fish.contains("!s / !camila"));
        assert!(!without_fish.contains("!dai"));
        assert!(!without_fish.contains("!jacinta"));
        assert!(voice_list(true).contains("!jacinta"));
        assert!(without_fish.contains("!s Hola stream"));
        assert!(without_fish.contains("500 caracteres"));
        assert!(without_fish.contains("15 s"));
    }

    #[test]
    fn low_latency_fish_audio_does_not_reuse_old_cache_or_change_edge_cache() {
        assert_ne!(cache_identity("Hola", "jacinta"), "jacinta:Hola");
        assert_eq!(
            cache_identity("Hola", "jacinta"),
            fish_audio::cache_identity("Hola")
        );
        assert_eq!(cache_identity("Hola", "camila"), "camila:Hola");
    }

    #[tokio::test]
    async fn chat_voices_announce_author_without_truncating_message_or_changing_alerts() {
        let (channel, mut receiver) = crate::test_support::channel("alpha", 1);
        for &(voice, _) in edge_tts::VOICES.iter().chain(std::iter::once(&(
            fish_audio::VOICE_ALIAS,
            fish_audio::MODEL_ID,
        ))) {
            assert!(enqueue_chat(&channel, "Senior_Dai", " Hola mundo ", voice));
            let item = receiver.recv().await.unwrap();
            assert_eq!(item.text, "Senior Dai dice: Hola mundo");
            assert_eq!(item.voice, voice);
        }
        let message = "ñ".repeat(KICK_MESSAGE_MAX);
        assert!(enqueue_chat(&channel, &"a".repeat(60), &message, "camila"));
        let item = receiver.recv().await.unwrap();
        assert_eq!(item.text, format!("{} dice: {message}", "a".repeat(40)));
        assert_eq!(item.text.chars().count(), 547);
        assert!(!enqueue_chat(
            &channel,
            "viewer",
            &"x".repeat(KICK_MESSAGE_MAX + 1),
            "camila"
        ));
        assert!(!enqueue_chat(&channel, "viewer", " ", "camila"));
        assert!(!enqueue_chat(&channel, "viewer", "Hola", "unknown"));
        assert!(enqueue_chat(&channel, "___", "Hola", "camila"));
        assert_eq!(
            receiver.recv().await.unwrap().text,
            "Un participante dice: Hola"
        );
        assert!(enqueue(&channel, "Gracias por el follow", "dalia"));
        assert_eq!(receiver.recv().await.unwrap().text, "Gracias por el follow");
    }

    #[tokio::test]
    async fn chat_command_aliases_all_include_the_sender() {
        let pool = sqlx::postgres::PgPoolOptions::new()
            .connect_lazy("postgres://unused:unused@127.0.0.1/unused")
            .unwrap();
        let app = crate::test_support::app(pool);
        let (channel, mut receiver) = crate::test_support::channel("alpha", 1);
        *channel.channel_id.write().await = None;
        for (command, voice) in [
            ("!s", "camila"),
            ("!camila", "camila"),
            ("!dalia", "dalia"),
            ("!jorge", "jorge"),
            ("!alex", "alex"),
            ("!narrador", "narrador"),
            ("!epico", "epico"),
            ("!comedia", "comedia"),
            ("!jacinta", fish_audio::VOICE_ALIAS),
        ] {
            crate::commands::handle("Ana_7", &format!("{command} Hola"), true, &channel, &app)
                .await;
            let item = receiver.try_recv().unwrap();
            assert_eq!(item.text, "Ana 7 dice: Hola");
            assert_eq!(item.voice, voice);
        }
        crate::commands::handle("Ana_7", "!dai Hola", true, &channel, &app).await;
        assert!(receiver.try_recv().is_err());
    }
    #[tokio::test]
    async fn tts_rejects_long_invalid_and_excess_requests() {
        let (channel, _receiver) = crate::test_support::channel("alpha", 1);
        assert!(!enqueue(
            &channel,
            &"x".repeat(KICK_MESSAGE_MAX + 1),
            "camila"
        ));
        assert!(!enqueue(&channel, "hello", "unknown"));
        assert!(!enqueue(&channel, " ", "camila"));
        for _ in 0..32 {
            assert!(enqueue(&channel, "hello", "camila"));
        }
        assert!(!enqueue(&channel, "overflow", "camila"));
        channel.cancel.cancel();
        assert!(!enqueue(&channel, "cancelled", "camila"));
    }
}
