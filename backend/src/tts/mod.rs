pub mod edge_tts;
pub mod fish_audio;
use base64::{engine::general_purpose::STANDARD, Engine};
use std::{path::PathBuf, sync::Arc};
use tokio::sync::{mpsc, Semaphore};

pub struct TtsQueueItem {
    pub text: String,
    pub voice: String,
}
pub fn enqueue(ch: &crate::state::ChannelState, text: &str, voice: &str) -> bool {
    enqueue_with_limit(ch, text, voice, 350)
}

pub fn enqueue_chat(
    ch: &crate::state::ChannelState,
    username: &str,
    text: &str,
    voice: &str,
) -> bool {
    if text.trim().is_empty() || text.chars().count() > 350 {
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
    enqueue_with_limit(ch, &format!("{name} dice: {}", text.trim()), voice, 397)
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
    pub async fn generate(&self, text: &str, voice: &str) -> Option<String> {
        use sha2::{Digest, Sha256};
        let hash = Sha256::digest(format!("{voice}:{text}"));
        let path = self.cache_dir.join(format!("{hash:x}.mp3"));
        tokio::fs::create_dir_all(&self.cache_dir).await.ok()?;
        self.prune().await;
        if let Ok(bytes) = tokio::fs::read(&path).await {
            return Some(STANDARD.encode(bytes));
        }
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
                let _ = tokio::fs::write(path, &bytes).await;
                Some(STANDARD.encode(bytes))
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
    slug: String,
    slots: Arc<Semaphore>,
) {
    while let Some(item) = rx.recv().await {
        let Ok(_permit) = slots.acquire().await else {
            return;
        };
        if let Some(audio) = service.generate(&item.text, &item.voice).await {
            io.to(slug.clone())
                .emit("speak", serde_json::json!({"audioBase64":audio}))
                .ok();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
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
        let message = "ñ".repeat(350);
        assert!(enqueue_chat(&channel, &"a".repeat(60), &message, "camila"));
        let item = receiver.recv().await.unwrap();
        assert_eq!(item.text, format!("{} dice: {message}", "a".repeat(40)));
        assert_eq!(item.text.chars().count(), 397);
        assert!(!enqueue_chat(
            &channel,
            "viewer",
            &"x".repeat(351),
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
            ("!dai", "camila"),
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
    }
    #[tokio::test]
    async fn tts_rejects_long_invalid_and_excess_requests() {
        let (channel, _receiver) = crate::test_support::channel("alpha", 1);
        assert!(!enqueue(&channel, &"x".repeat(351), "camila"));
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
