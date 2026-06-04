use crate::{
    kick::sender,
    queue::VideoItem,
    server::ns_emit,
    state::{AppState, ChannelState},
    tts,
};
use serde_json::json;
use std::sync::Arc;
use tracing::{info, warn};

const YT_API_KEY: &str = "AIzaSyBJRSpiY0bvQmjmJDdvUNPLRU_Z4YNCrRs";

pub async fn handle(username: &str, content: &str, ch: &Arc<ChannelState>, global: &Arc<AppState>) {
    info!("[CHAT][{}] {username}: {content}", ch.slug);
    let is_owner = username.eq_ignore_ascii_case(&ch.slug);
    let cmd = content.to_lowercase();

    // ── Comandos solo del owner ───────────────────────────────────────────────
    if is_owner {
        match cmd.as_str() {
            "!von"  => { ns_emit(global, &ch.slug, "toggleVideo", json!({"showVideo": true}));  return; }
            "!voff" => { ns_emit(global, &ch.slug, "toggleVideo", json!({"showVideo": false})); return; }
            "!vstop" => {
                let mut q = ch.video_queue.write().await;
                q.clear(); drop(q);
                ns_emit(global, &ch.slug, "syncQueue", json!({"items": []}));
                return;
            }
            "!next" | "!skip" => {
                ns_emit(global, &ch.slug, "nextVideo", json!({}));
                return;
            }
            _ => {}
        }
    }

    // ── Comandos informativos (cooldown 20s) ──────────────────────────────────
    macro_rules! global_cmd {
        ($key:expr, $body:expr) => {{
            if !is_owner {
                let ok = { ch.cooldown.lock().await.check_global($key, 20) };
                if !ok { return; }
                ch.cooldown.lock().await.use_global($key);
            }
            $body
            return;
        }};
    }

    match cmd.as_str() {
        "!discord" => global_cmd!("!discord", {
            if !ch.commands.discord.is_empty() {
                sender::send(&format!("💬 Discord → {}", ch.commands.discord), ch, global).await;
            }
        }),
        "!redes" | "!rrss" | "!rss" => global_cmd!("!redes", {
            if !ch.commands.redes.is_empty() {
                sender::send(&format!("📱 Redes → {}", ch.commands.redes), ch, global).await;
            }
        }),
        "!pc" | "!setup" | "!specs" => global_cmd!("!pc", {
            if !ch.commands.pc.is_empty() {
                sender::send(&format!("🖥️ Setup → {}", ch.commands.pc), ch, global).await;
            }
        }),
        "!horario" | "!schedule" => global_cmd!("!horario", {
            if !ch.commands.horario.is_empty() {
                sender::send(&format!("📅 Horario → {}", ch.commands.horario), ch, global).await;
            }
        }),
        "!comandos" | "!help" | "!ayuda" | "!commands" => global_cmd!("!comandos", {
            sender::send(
                "📋 Comandos: !play [url] · !dai/!dalia/!jorge/!alex [texto TTS] · !quitarme · !misongs · !dado · !8ball [pregunta] · !sorteo · !uptime · !cola · !discord · !redes · !pc · !horario",
                ch, global,
            ).await;
        }),
        _ => {}
    }

    // ── Comandos dinámicos ────────────────────────────────────────────────────
    use std::sync::atomic::Ordering;

    match cmd.as_str() {
        "!uptime" => {
            if !is_owner {
                let ok = { ch.cooldown.lock().await.check_global("!uptime", 30) };
                if !ok { return; }
                ch.cooldown.lock().await.use_global("!uptime");
            }
            let secs = ch.start_time.elapsed().as_secs();
            let msg = if secs / 3600 > 0 {
                format!("⏱️ Llevamos {}h {}m en vivo", secs / 3600, (secs % 3600) / 60)
            } else {
                format!("⏱️ Llevamos {}m en vivo", secs / 60)
            };
            sender::send(&msg, ch, global).await;
            return;
        }
        "!cola" | "!queue" => {
            if !is_owner {
                let ok = { ch.cooldown.lock().await.check_global("!cola", 30) };
                if !ok { return; }
                ch.cooldown.lock().await.use_global("!cola");
            }
            let q = ch.video_queue.read().await;
            let msg = if q.items.is_empty() {
                "📭 La cola de videos está vacía".to_string()
            } else {
                let lista: Vec<String> = q.items.iter().enumerate().take(5)
                    .map(|(i, v)| format!("{}. {}", i + 1, v.title))
                    .collect();
                let extra = if q.items.len() > 5 { format!(" (+{} más)", q.items.len() - 5) } else { String::new() };
                format!("🎬 Cola: {}{}", lista.join(" · "), extra)
            };
            sender::send(&msg, ch, global).await;
            return;
        }
        "!seguidores" | "!followers" => {
            if !is_owner {
                let ok = { ch.cooldown.lock().await.check_global("!seguidores", 60) };
                if !ok { return; }
                ch.cooldown.lock().await.use_global("!seguidores");
            }
            let actual = ch.followers.load(Ordering::Relaxed);
            let pct = if ch.follow_goal > 0 { actual * 100 / ch.follow_goal } else { 0 };
            sender::send(&format!("👥 Seguidores: {actual} / {} ({pct}%)", ch.follow_goal), ch, global).await;
            return;
        }
        _ => {}
    }

    // ── Entretenimiento ───────────────────────────────────────────────────────
    use rand::Rng;
    use rand::seq::SliceRandom;

    if cmd == "!dado" {
        if !is_owner {
            let ok = { ch.cooldown.lock().await.check_user(username, "!dado", 15) };
            if !ok { return; }
            ch.cooldown.lock().await.use_user(username, "!dado");
        }
        let n: u8 = rand::thread_rng().gen_range(1..=100);
        sender::send(&format!("🎲 {username} sacó un {n}!"), ch, global).await;
        return;
    }

    if cmd == "!8ball" || cmd.starts_with("!8ball ") {
        if !is_owner {
            let ok = { ch.cooldown.lock().await.check_user(username, "!8ball", 10) };
            if !ok { return; }
            ch.cooldown.lock().await.use_user(username, "!8ball");
        }
        const RESP: &[&str] = &[
            "Sí, definitivamente 🟢","Es cierto 🟢","Sin duda 🟢","Por supuesto 🟢",
            "Probablemente sí 🟡","Perspectivas favorables 🟡",
            "No lo sé, pregunta más tarde 🔵","Mejor no te digo ahora 🔵",
            "No cuentes con ello 🔴","Mi respuesta es no 🔴","Muy dudoso 🔴",
        ];
        let resp = RESP.choose(&mut rand::thread_rng()).unwrap_or(&"Quizás");
        sender::send(&format!("🎱 {resp}"), ch, global).await;
        return;
    }

    // !sorteo
    if cmd == "!sorteo" || cmd.starts_with("!sorteo ") || cmd == "!participar" || cmd == "!entrar" {
        let sub = if cmd == "!participar" || cmd == "!entrar" { "" }
                  else { cmd.strip_prefix("!sorteo").unwrap_or("").trim() };
        match sub {
            "abrir" | "open" | "start" if is_owner => {
                let mut s = ch.sorteo.lock().await;
                s.open = true; s.participants.clear(); drop(s);
                sender::send("🎟️ ¡El sorteo está abierto! Escribe !participar para unirte", ch, global).await;
            }
            "cerrar" | "close" | "stop" if is_owner => {
                let mut s = ch.sorteo.lock().await;
                s.open = false;
                let count = s.participants.len(); drop(s);
                sender::send(&format!("🔒 Sorteo cerrado. {count} participantes"), ch, global).await;
            }
            "ganador" | "winner" if is_owner => {
                let mut s = ch.sorteo.lock().await;
                s.open = false;
                let winner = s.participants.choose(&mut rand::thread_rng()).cloned(); drop(s);
                match winner {
                    Some(w) => sender::send(&format!("🏆 ¡El ganador es @{w}! 🎉"), ch, global).await,
                    None    => sender::send("😅 No hay participantes", ch, global).await,
                }
            }
            "" => {
                let mut s = ch.sorteo.lock().await;
                if !s.open || s.participants.iter().any(|p| p.eq_ignore_ascii_case(username)) { return; }
                s.participants.push(username.to_string());
                let count = s.participants.len(); drop(s);
                sender::send(&format!("✅ @{username} se unió al sorteo! ({count})"), ch, global).await;
            }
            _ => {}
        }
        return;
    }

    // ── Gestión de cola ───────────────────────────────────────────────────────
    if cmd == "!quitarme" || cmd == "!removeme" {
        let mut q = ch.video_queue.write().await;
        if let Some(pos) = q.items.iter().position(|v| v.user.eq_ignore_ascii_case(username)) {
            let title = q.items[pos].title.clone();
            q.remove(pos);
            let items = q.items.clone(); drop(q);
            ns_emit(global, &ch.slug, "syncQueue", json!({"items": &items}));
            sender::send(&format!("🗑️ @{username} eliminó '{title}' de la cola"), ch, global).await;
        }
        return;
    }

    if cmd == "!misongs" || cmd == "!miscanciones" {
        if !is_owner {
            let ok = { ch.cooldown.lock().await.check_user(username, "!misongs", 15) };
            if !ok { return; }
            ch.cooldown.lock().await.use_user(username, "!misongs");
        }
        let q = ch.video_queue.read().await;
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
        sender::send(&msg, ch, global).await;
        return;
    }

    // ── !play ─────────────────────────────────────────────────────────────────
    if let Some(url) = content.strip_prefix("!play ") {
        if !is_owner {
            let ok = { ch.cooldown.lock().await.check_user(username, "!play", 30) };
            if !ok { return; }
            ch.cooldown.lock().await.use_user(username, "!play");
        }
        play(url.trim().to_string(), username.to_string(), ch, global).await;
        return;
    }

    // ── TTS ───────────────────────────────────────────────────────────────────
    let Some((cmd_word, rest)) = content.split_once(' ') else { return };
    let text = rest.trim();
    if text.is_empty() { return; }
    let cmd_low = cmd_word.to_lowercase();
    let is_tts = cmd_low == "!dai"
        || cmd_low.strip_prefix('!').map_or(false, |v| tts::edge_tts::is_valid_voice(v));
    if is_tts {
        if !is_owner {
            let ok = { ch.cooldown.lock().await.check_user(username, "!dai", 15) };
            if !ok { return; }
            ch.cooldown.lock().await.use_user(username, "!dai");
        }
        let voice = if cmd_low == "!dai" { "camila".into() }
                    else { cmd_low.trim_start_matches('!').to_string() };
        ch.tts_tx.send(tts::TtsQueueItem { text: text.into(), voice }).ok();
    }
}

pub async fn play(url: String, username: String, ch: &Arc<ChannelState>, global: &Arc<AppState>) {
    info!("[PLAY][{}] {username}: {url}", ch.slug);

    if is_direct_video(&url) {
        let title = url.split('/').next_back()
            .and_then(|s| s.split('?').next()).unwrap_or("Video").to_string();
        let msg = format!("▶ @{username} agregó «{}» a la cola", trunc(&title, 60));
        enqueue(VideoItem { video_id: None, url: Some(url), title, user: username }, ch, global).await;
        sender::send(&msg, ch, global).await;
        return;
    }

    if url.contains("list=") {
        let list_id = url.split("list=").nth(1).unwrap_or("").split('&').next().unwrap_or("");
        if !list_id.starts_with("RD") {
            if let Some((vid, title)) = first_from_playlist(&global.http, &url).await {
                let msg = format!("▶ @{username} agregó «{}» a la cola", trunc(&title, 60));
                enqueue(VideoItem { video_id: Some(vid), url: None, title, user: username }, ch, global).await;
                sender::send(&msg, ch, global).await;
                return;
            }
        }
    }

    if let Some(vid) = yt_id(&url) {
        let title = yt_title(&global.http, &url).await;
        let msg = format!("▶ @{username} agregó «{}» a la cola", trunc(&title, 60));
        enqueue(VideoItem { video_id: Some(vid), url: None, title, user: username }, ch, global).await;
        sender::send(&msg, ch, global).await;
    } else {
        warn!("[PLAY] URL inválida: {url}");
        sender::send(&format!("@{username} no pude procesar esa URL."), ch, global).await;
    }
}

async fn enqueue(item: VideoItem, ch: &Arc<ChannelState>, global: &Arc<AppState>) {
    let title = item.title.clone();
    let mut q = ch.video_queue.write().await;
    q.push(item);
    let items = q.items.clone(); drop(q);
    info!("[PLAY][{}] Cola: {title}", ch.slug);
    ns_emit(global, &ch.slug, "syncQueue", json!({"items": &items}));
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
            let id: String = url[pos + sep.len()..].chars()
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
        "https://www.googleapis.com/youtube/v3/playlistItems?part=snippet&maxResults=1&playlistId={list_id}&key={YT_API_KEY}"
    );
    let json: serde_json::Value = http.get(&api).send().await.ok()?.json().await.ok()?;
    let item = json["items"].as_array()?.first()?;
    let vid   = item["snippet"]["resourceId"]["videoId"].as_str()?.to_string();
    let title = item["snippet"]["title"].as_str().unwrap_or("Video").to_string();
    Some((vid, title))
}

fn trunc(s: &str, max_chars: usize) -> &str {
    match s.char_indices().nth(max_chars) {
        Some((i, _)) => &s[..i],
        None         => s,
    }
}

async fn yt_title(http: &reqwest::Client, url: &str) -> String {
    let enc: String = url.bytes().flat_map(|b| {
        if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.' | b'~') { vec![b as char] }
        else { format!("%{b:02X}").chars().collect() }
    }).collect();
    let api = format!("https://noembed.com/embed?url={enc}");
    let Ok(resp) = http.get(&api).send().await else { return "Video de YouTube".to_string() };
    let Ok(json) = resp.json::<serde_json::Value>().await else { return "Video de YouTube".to_string() };
    json["title"].as_str().unwrap_or("Video de YouTube").to_string()
}
