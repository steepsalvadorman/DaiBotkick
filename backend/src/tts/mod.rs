pub mod edge_tts;

use base64::{engine::general_purpose::STANDARD as B64, Engine};
use std::{
    collections::hash_map::DefaultHasher,
    fs,
    hash::{Hash, Hasher},
    path::PathBuf,
    sync::Arc,
};
use tokio::sync::mpsc;
use tracing::{info, warn};

pub struct TtsQueueItem {
    pub text:  String,
    pub voice: String,
}

pub struct TtsService {
    cache_dir: PathBuf,
}

impl TtsService {
    pub fn new(cache_dir: &str) -> Self {
        let path = PathBuf::from(cache_dir);
        fs::create_dir_all(&path).ok();
        Self { cache_dir: path }
    }

    fn cache_path(&self, voice: &str, text: &str) -> PathBuf {
        let mut h = DefaultHasher::new();
        format!("{voice}:{text}").hash(&mut h);
        self.cache_dir.join(format!("{:016x}.mp3", h.finish()))
    }

    pub async fn generate(&self, text: &str, voice: &str) -> Option<String> {
        let cache = self.cache_path(voice, text);
        if let Ok(bytes) = fs::read(&cache) {
            info!("[TTS] Caché: \"{:.40}\"", text);
            return Some(B64.encode(&bytes));
        }
        match edge_tts::synthesize(text, voice).await {
            Ok(bytes) => {
                let _ = fs::write(&cache, &bytes);
                info!("[TTS OK] \"{:.40}\"", text);
                Some(B64.encode(&bytes))
            }
            Err(e) => { warn!("[TTS ERR] {e}"); None }
        }
    }
}

/// Arranca el procesador TTS para un canal. Emite a la room del canal.
pub async fn spawn_processor(
    service: Arc<TtsService>,
    mut rx:  mpsc::UnboundedReceiver<TtsQueueItem>,
    io:      socketioxide::SocketIo,
    slug:    String,
) {
    while let Some(item) = rx.recv().await {
        if let Some(b64) = service.generate(&item.text, &item.voice).await {
            io.to(slug.clone())
              .emit("speak", serde_json::json!({ "audioBase64": b64 }))
              .ok();
        }
    }
}
