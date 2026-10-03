pub mod edge_tts;
use base64::{engine::general_purpose::STANDARD, Engine};
use std::{path::PathBuf, sync::Arc};
use tokio::sync::{mpsc, Semaphore};

pub struct TtsQueueItem {
    pub text: String,
    pub voice: String,
}
pub fn enqueue(ch: &crate::state::ChannelState, text: &str, voice: &str) -> bool {
    if ch.cancel.is_cancelled()
        || text.trim().is_empty()
        || text.chars().count() > 350
        || !edge_tts::is_valid_voice(voice)
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

pub struct TtsService {
    cache_dir: PathBuf,
}
impl TtsService {
    pub fn new(cache_dir: &str) -> Self {
        Self {
            cache_dir: PathBuf::from(cache_dir),
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
        match edge_tts::synthesize(text, voice).await {
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
