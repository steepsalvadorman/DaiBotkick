use crate::{queue::VideoItem, tts, AppState};
use serde_json::json;
use std::sync::Arc;
use tracing::{info, warn};

const YT_API_KEY: &str = "AIzaSyBJRSpiY0bvQmjmJDdvUNPLRU_Z4YNCrRs";

/// Punto de entrada — llamado desde kick/mod.rs y desde panelCommand.
pub async fn handle(username: &str, content: &str, state: &Arc<AppState>) {
    info!("[CHAT] {username}: {content}");
    let is_owner = username.eq_ignore_ascii_case(&state.config.channel_name);
    let cmd = content.to_lowercase();

    // ── Owner ──────────────────────────────────────────────────────────────────
    if is_owner {
        match cmd.as_str() {
            "!von" => {
                info!("[CMD] {username}: !von");
                state.io.emit("toggleVideo", json!({"showVideo": true})).ok();
                return;
            }
            "!voff" => {
                info!("[CMD] {username}: !voff");
                state.io.emit("toggleVideo", json!({"showVideo": false})).ok();
                return;
            }
            "!vstop" => {
                info!("[CMD] {username}: !vstop");
                let mut q = state.video_queue.write().await;
                q.clear();
                drop(q);
                state.io.emit("syncQueue", json!({"items": []})).ok();
                return;
            }
            "!next" | "!skip" => {
                info!("[CMD] {username}: !next");
                state.io.emit("nextVideo", json!({})).ok();
                return;
            }
            _ => {}
        }
    }

    // ── Comandos informativos (cooldown global 20s) ───────────────────────────
    {
        use crate::kick::sender;
        let cfg = &state.config;

        macro_rules! global_cmd {
            ($key:expr, $body:expr) => {{
                if !is_owner {
                    let ok = { state.cooldown.lock().await.check_global($key, 20) };
                    if !ok { return; }
                    state.cooldown.lock().await.use_global($key);
                }
                $body
                return;
            }};
        }

        match cmd.as_str() {
            "!discord" => global_cmd!("!discord", {
                if !cfg.cmd_discord.is_empty() {
                    sender::send(&format!("💬 Discord → {}", cfg.cmd_discord), state).await;
                }
            }),
            "!redes" | "!rrss" | "!rss" => global_cmd!("!redes", {
                if !cfg.cmd_redes.is_empty() {
                    sender::send(&format!("📱 Redes → {}", cfg.cmd_redes), state).await;
                }
            }),
            "!pc" | "!setup" | "!specs" => global_cmd!("!pc", {
                if !cfg.cmd_pc.is_empty() {
                    sender::send(&format!("🖥️ Setup → {}", cfg.cmd_pc), state).await;
                }
            }),
            "!horario" | "!schedule" => global_cmd!("!horario", {
                if !cfg.cmd_horario.is_empty() {
                    sender::send(&format!("📅 Horario → {}", cfg.cmd_horario), state).await;
                }
            }),
            "!comandos" | "!help" | "!ayuda" | "!commands" => global_cmd!("!comandos", {
                sender::send(
                    "📋 Comandos: !play [url] · !s [texto] · !quitarme · !misongs · !dado · !8ball [pregunta] · !sorteo · !uptime · !cola · !discord · !redes · !pc · !horario",
                    state,
                ).await;
            }),
            _ => {}
        }
    }

    // ── Comandos dinámicos (cooldown global 30-60s) ───────────────────────────
    {
        use crate::kick::sender;
        use std::sync::atomic::Ordering;

        match cmd.as_str() {
            "!uptime" => {
                if !is_owner {
                    let ok = { state.cooldown.lock().await.check_global("!uptime", 30) };
                    if !ok { return; }
                    state.cooldown.lock().await.use_global("!uptime");
                }
                let secs  = state.start_time.elapsed().as_secs();
                let horas = secs / 3600;
                let mins  = (secs % 3600) / 60;
                let msg   = if horas > 0 {
                    format!("⏱️ Llevamos {horas}h {mins}m en vivo")
                } else {
                    format!("⏱️ Llevamos {mins}m en vivo")
                };
                sender::send(&msg, state).await;
                return;
            }
            "!cola" | "!queue" => {
                if !is_owner {
                    let ok = { state.cooldown.lock().await.check_global("!cola", 30) };
                    if !ok { return; }
                    state.cooldown.lock().await.use_global("!cola");
                }
                let q = state.video_queue.read().await;
                let msg = if q.items.is_empty() {
                    "📭 La cola de videos está vacía".to_string()
                } else {
                    let lista: Vec<String> = q.items.iter().enumerate()
                        .take(5)
                        .map(|(i, v)| format!("{}. {}", i + 1, v.title))
                        .collect();
                    let extra = if q.items.len() > 5 {
                        format!(" (+{} más)", q.items.len() - 5)
                    } else {
                        String::new()
                    };
                    format!("🎬 Cola: {}{}", lista.join(" · "), extra)
                };
                sender::send(&msg, state).await;
                return;
            }
            "!seguidores" | "!followers" | "!seguidos" => {
                if !is_owner {
                    let ok = { state.cooldown.lock().await.check_global("!seguidores", 60) };
                    if !ok { return; }
                    state.cooldown.lock().await.use_global("!seguidores");
                }
                let actual = state.followers.load(Ordering::Relaxed);
                let meta   = state.config.follow_goal;
                let pct    = if meta > 0 { actual * 100 / meta } else { 0 };
                sender::send(
                    &format!("👥 Seguidores: {actual} / {meta} ({pct}%)"),
                    state,
                ).await;
                return;
            }
            _ => {}
        }
    }

    // ── Entretenimiento ───────────────────────────────────────────────────────
    {
        use crate::kick::sender;
        use rand::Rng;
        use rand::seq::SliceRandom;

        // !dado — cooldown 15s por usuario
        if cmd == "!dado" {
            if !is_owner {
                let ok = { state.cooldown.lock().await.check_user(username, "!dado", 15) };
                if !ok { return; }
                state.cooldown.lock().await.use_user(username, "!dado");
            }
            let n: u8 = rand::thread_rng().gen_range(1..=100);
            sender::send(&format!("🎲 {username} sacó un {n}!"), state).await;
            return;
        }

        // !8ball — cooldown 10s por usuario
        if cmd == "!8ball" || cmd.starts_with("!8ball ") {
            if !is_owner {
                let ok = { state.cooldown.lock().await.check_user(username, "!8ball", 10) };
                if !ok { return; }
                state.cooldown.lock().await.use_user(username, "!8ball");
            }
            const RESPUESTAS: &[&str] = &[
                "Sí, definitivamente 🟢", "Es cierto 🟢", "Sin duda 🟢",
                "Por supuesto 🟢", "Puedes contar con ello 🟢",
                "Probablemente sí 🟡", "Perspectivas favorables 🟡",
                "Las señales apuntan al sí 🟡",
                "No lo sé, pregunta más tarde 🔵", "Concéntrate y pregunta de nuevo 🔵",
                "Mejor no te digo ahora 🔵", "Difícil de decir 🔵",
                "No cuentes con ello 🔴", "Mi respuesta es no 🔴",
                "Las perspectivas no son buenas 🔴", "Muy dudoso 🔴",
            ];
            let resp = RESPUESTAS.choose(&mut rand::thread_rng()).unwrap_or(&"Quizás");
            sender::send(&format!("🎱 {resp}"), state).await;
            return;
        }

        // !sorteo
        if cmd == "!sorteo" || cmd.starts_with("!sorteo ") || cmd == "!participar" || cmd == "!entrar" {
            let sub = if cmd == "!participar" || cmd == "!entrar" {
                ""
            } else {
                cmd.strip_prefix("!sorteo").unwrap_or("").trim()
            };

            match sub {
                "abrir" | "open" | "start" if is_owner => {
                    let mut s = state.sorteo.lock().await;
                    s.open = true;
                    s.participants.clear();
                    drop(s);
                    sender::send("🎟️ ¡El sorteo está abierto! Escribe !sorteo o !participar para unirte", state).await;
                }
                "cerrar" | "close" | "stop" if is_owner => {
                    let mut s = state.sorteo.lock().await;
                    s.open = false;
                    let count = s.participants.len();
                    drop(s);
                    sender::send(&format!("🔒 Sorteo cerrado. {count} participantes registrados"), state).await;
                }
                "ganador" | "winner" | "resultado" if is_owner => {
                    let mut s = state.sorteo.lock().await;
                    s.open = false;
                    let winner = s.participants.choose(&mut rand::thread_rng()).cloned();
                    drop(s);
                    match winner {
                        Some(w) => sender::send(&format!("🏆 ¡¡El ganador del sorteo es @{w}!! 🎉🎉🎉"), state).await,
                        None    => sender::send("😅 No hay participantes en el sorteo", state).await,
                    }
                }
                "" => {
                    let mut s = state.sorteo.lock().await;
                    if !s.open { return; }
                    if s.participants.iter().any(|p| p.eq_ignore_ascii_case(username)) {
                        return;
                    }
                    s.participants.push(username.to_string());
                    let count = s.participants.len();
                    drop(s);
                    sender::send(&format!("✅ @{username} se unió al sorteo! ({count} participantes)"), state).await;
                }
                _ => {}
            }
            return;
        }
    }

    // ── Gestión personal de cola ──────────────────────────────────────────────
    {
        use crate::kick::sender;

        // !quitarme — elimina el primer video del usuario en la cola
        if cmd == "!quitarme" || cmd == "!removeme" || cmd == "!remove" {
            let mut q = state.video_queue.write().await;
            if let Some(pos) = q.items.iter().position(|v| v.user.eq_ignore_ascii_case(username)) {
                let title = q.items[pos].title.clone();
                q.remove(pos);
                let items = q.items.clone();
                drop(q);
                state.io.emit("syncQueue", json!({"items": &items})).ok();
                sender::send(&format!("🗑️ @{username} eliminó '{title}' de la cola"), state).await;
            }
            return;
        }

        // !misongs — lista los videos del usuario en la cola (cooldown 15s por usuario)
        if cmd == "!misongs" || cmd == "!miscanciones" || cmd == "!mivideos" {
            if !is_owner {
                let ok = { state.cooldown.lock().await.check_user(username, "!misongs", 15) };
                if !ok { return; }
                state.cooldown.lock().await.use_user(username, "!misongs");
            }
            let q = state.video_queue.read().await;
            let mis: Vec<String> = q.items.iter().enumerate()
                .filter(|(_, v)| v.user.eq_ignore_ascii_case(username))
                .map(|(i, v)| format!("{}. {}", i + 1, v.title))
                .collect();
            drop(q);
            let msg = if mis.is_empty() {
                format!("@{username} no tienes videos en la cola")
            } else {
                format!("🎵 @{username}: {}", mis.join(" · "))
            };
            sender::send(&msg, state).await;
            return;
        }
    }

    // ── !play — cooldown 30s por usuario ──────────────────────────────────────
    if let Some(url) = content.strip_prefix("!play ") {
        if !is_owner {
            let ok = { state.cooldown.lock().await.check_user(username, "!play", 30) };
            if !ok { return; }
            state.cooldown.lock().await.use_user(username, "!play");
        }
        play(url.trim().to_string(), username.to_string(), state).await;
        return;
    }

    // ── Voz (!s y voces directas) — cooldown 15s por usuario ─────────────────
    let Some((cmd_word, rest)) = content.split_once(' ') else { return };
    let text = rest.trim();
    if text.is_empty() { return; }

    let cmd_low = cmd_word.to_lowercase();
    let is_tts = cmd_low == "!s"
        || cmd_low.strip_prefix('!').map_or(false, |v| tts::edge_tts::is_valid_voice(v));

    if is_tts {
        if !is_owner {
            let ok = { state.cooldown.lock().await.check_user(username, "!s", 15) };
            if !ok { return; }
            state.cooldown.lock().await.use_user(username, "!s");
        }
        let voice = if cmd_low == "!s" {
            "camila".to_string()
        } else {
            cmd_low.trim_start_matches('!').to_string()
        };
        state.tts_tx.send(tts::TtsQueueItem { text: text.into(), voice }).ok();
    }
}

/// Lógica de !play: YouTube, playlist, video directo.
pub async fn play(url: String, username: String, state: &Arc<AppState>) {
    use crate::kick::sender;
    info!("[PLAY] {username}: {url}");

    // 1. Video directo
    if is_direct_video(&url) {
        let title = url.split('/').next_back()
            .and_then(|s| s.split('?').next())
            .unwrap_or("Video Directo")
            .to_string();
        enqueue(VideoItem { video_id: None, url: Some(url), title, user: username }, state).await;
        return;
    }

    // 2. YouTube playlist (ignora las RD/mix — solo tomar el video individual)
    if url.contains("list=") {
        let list_id = url.split("list=").nth(1).unwrap_or("").split('&').next().unwrap_or("");
        if !list_id.starts_with("RD") {
            if let Some((vid, title)) = first_from_playlist(&state.http, &url).await {
                let msg = format!("▶ @{username} agregó «{}» a la cola", &title[..title.len().min(60)]);
                enqueue(VideoItem { video_id: Some(vid), url: None, title, user: username }, state).await;
                sender::send(&msg, state).await;
                return;
            }
        }
        // Es un mix/RD o falló la playlist → tratar como video individual
    }

    // 3. YouTube individual
    if let Some(vid) = yt_id(&url) {
        let title = yt_title(&state.http, &url).await;
        let msg = format!("▶ @{username} agregó «{}» a la cola", &title[..title.len().min(60)]);
        enqueue(VideoItem { video_id: Some(vid), url: None, title, user: username }, state).await;
        sender::send(&msg, state).await;
    } else {
        warn!("[PLAY] No se pudo extraer ID: {url}");
        sender::send(
            &format!("@{username} no pude procesar esa URL. Usa un link de YouTube válido."),
            state,
        ).await;
    }
}

async fn enqueue(item: VideoItem, state: &Arc<AppState>) {
    let title = item.title.clone();
    let mut q = state.video_queue.write().await;
    q.push(item);
    let items = q.items.clone();
    drop(q);
    info!("[PLAY] Cola: {title}");
    state.io.emit("syncQueue", serde_json::json!({"items": &items})).ok();
}

fn is_direct_video(url: &str) -> bool {
    let path = url.split('?').next().unwrap_or(url).to_lowercase();
    matches!(
        std::path::Path::new(&path).extension().and_then(|e| e.to_str()),
        Some("mp4" | "webm" | "mov" | "m4v")
    )
}

fn yt_id(url: &str) -> Option<String> {
    for sep in &["youtu.be/", "?v=", "&v=", "/embed/", "/shorts/", "/watch?v="] {
        if let Some(pos) = url.find(sep) {
            let id: String = url[pos + sep.len()..]
                .chars()
                .take_while(|c| !matches!(c, '?' | '&' | '"' | '\'' | '>' | ' '))
                .collect();
            if id.len() >= 8 { return Some(id); }
        }
    }
    None
}

async fn first_from_playlist(http: &reqwest::Client, url: &str) -> Option<(String, String)> {
    let list_id = url.split("list=").nth(1)?.split('&').next()?;
    if list_id.starts_with("RD") { return None; }
    let api = format!(
        "https://www.googleapis.com/youtube/v3/playlistItems\
         ?part=snippet&maxResults=1&playlistId={list_id}&key={YT_API_KEY}"
    );
    let json: serde_json::Value = http.get(&api).send().await.ok()?.json().await.ok()?;
    let item = json["items"].as_array()?.first()?;
    let vid   = item["snippet"]["resourceId"]["videoId"].as_str()?.to_string();
    let title = item["snippet"]["title"].as_str().unwrap_or("Video").to_string();
    Some((vid, title))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn yt_id_standard_watch_url() {
        assert_eq!(yt_id("https://www.youtube.com/watch?v=dQw4w9WgXcQ"), Some("dQw4w9WgXcQ".into()));
    }

    #[test]
    fn yt_id_short_youtu_be() {
        assert_eq!(yt_id("https://youtu.be/dQw4w9WgXcQ"), Some("dQw4w9WgXcQ".into()));
    }

    #[test]
    fn yt_id_shorts_format() {
        assert_eq!(yt_id("https://www.youtube.com/shorts/dQw4w9WgXcQ"), Some("dQw4w9WgXcQ".into()));
    }

    #[test]
    fn yt_id_embed_format() {
        assert_eq!(yt_id("https://www.youtube.com/embed/dQw4w9WgXcQ"), Some("dQw4w9WgXcQ".into()));
    }

    #[test]
    fn yt_id_with_extra_params() {
        assert_eq!(
            yt_id("https://www.youtube.com/watch?v=dQw4w9WgXcQ&t=42&list=PL123"),
            Some("dQw4w9WgXcQ".into())
        );
    }

    #[test]
    fn yt_id_youtu_be_with_params() {
        assert_eq!(yt_id("https://youtu.be/dQw4w9WgXcQ?si=abc123"), Some("dQw4w9WgXcQ".into()));
    }

    #[test]
    fn yt_id_rejects_non_youtube() {
        assert_eq!(yt_id("https://www.twitch.tv/somestream"), None);
        assert_eq!(yt_id("https://open.spotify.com/track/abc"), None);
    }

    #[test]
    fn yt_id_rejects_too_short_id() {
        assert_eq!(yt_id("https://youtu.be/abc"), None); // menos de 8 chars
    }

    #[test]
    fn is_direct_video_detects_extensions() {
        assert!(is_direct_video("https://cdn.example.com/video.mp4"));
        assert!(is_direct_video("https://cdn.example.com/video.webm"));
        assert!(is_direct_video("https://cdn.example.com/video.mov"));
        assert!(is_direct_video("https://cdn.example.com/clip.mp4?token=abc"));
    }

    #[test]
    fn is_direct_video_rejects_other_urls() {
        assert!(!is_direct_video("https://www.youtube.com/watch?v=abc"));
        assert!(!is_direct_video("https://cdn.example.com/video.mp3"));
        assert!(!is_direct_video("https://cdn.example.com/file.html"));
    }
}

async fn yt_title(http: &reqwest::Client, url: &str) -> String {
    let enc: String = url.bytes().flat_map(|b| {
        if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.' | b'~') {
            vec![b as char]
        } else {
            format!("%{b:02X}").chars().collect()
        }
    }).collect();
    let api = format!("https://noembed.com/embed?url={enc}");
    http.get(&api).send().await.ok()
        .and_then(|r| tokio::task::block_in_place(|| {
            tokio::runtime::Handle::current().block_on(r.json::<serde_json::Value>()).ok()
        }))
        .and_then(|v| v["title"].as_str().map(|s| s.to_string()))
        .unwrap_or_else(|| "Video de YouTube".to_string())
}
