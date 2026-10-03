use crate::{
    kick::sender,
    queue::VideoItem,
    state::{AppState, ChannelState},
    tts,
};
use std::sync::Arc;
use tracing::warn;

pub async fn handle(
    username: &str,
    content: &str,
    is_owner: bool,
    ch: &Arc<ChannelState>,
    global: &Arc<AppState>,
) {
    if ch.cancel.is_cancelled() {
        return;
    }
    let cmd = content.to_lowercase();

    // ── Comandos solo del owner ───────────────────────────────────────────────
    if is_owner {
        if let Some(args) = command_args(content, "!set") {
            configure(args, ch, global).await;
            return;
        }
        match cmd.as_str() {
            "!von" => {
                visibility(ch, global, true).await;
                return;
            }
            "!voff" => {
                visibility(ch, global, false).await;
                return;
            }
            "!vstop" => {
                queue_edit(ch, global, |q| {
                    q.clear();
                    true
                })
                .await;
                return;
            }
            "!next" | "!skip" => {
                queue_edit(ch, global, |q| {
                    if q.items.is_empty() {
                        return false;
                    }
                    q.advance();
                    true
                })
                .await;
                return;
            }
            _ => {}
        }
    }

    // ── Comandos informativos (cooldown 20s) ──────────────────────────────────
    macro_rules! global_cmd {
        ($key:expr, $body:expr) => {{
            if !is_owner {
                let ok = { ch.cooldown.lock().await.consume_global($key, 20) };
                if !ok { return; }
            }
            $body
            return;
        }};
    }

    match cmd.as_str() {
        "!discord" => global_cmd!("!discord", {
            info(ch, global, "discord", "💬 Discord").await;
        }),
        "!redes" | "!rrss" | "!rss" => global_cmd!("!redes", {
            info(ch, global, "redes", "📱 Redes").await;
        }),
        "!pc" | "!setup" | "!specs" => global_cmd!("!pc", {
            info(ch, global, "pc", "🖥️ Setup").await;
        }),
        "!horario" | "!schedule" => global_cmd!("!horario", {
            info(ch, global, "horario", "📅 Horario").await;
        }),
        "!comandos" | "!help" | "!ayuda" | "!commands" => global_cmd!("!comandos", {
            sender::send(
                "📋 Comandos: !play [url] · !s/!dalia/!jorge/!alex [texto TTS] · !quitarme · !misongs · !dado · !8ball [pregunta] · !sorteo · !uptime · !cola · !discord · !redes · !pc · !horario",
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
                let ok = { ch.cooldown.lock().await.consume_global("!uptime", 30) };
                if !ok {
                    return;
                }
            }
            let since = *ch.live_since.read().await;
            let msg = match since {
                Some(start) => {
                    let secs = (chrono::Utc::now() - start).num_seconds().max(0);
                    format!("En vivo: {}h {}m", secs / 3600, (secs % 3600) / 60)
                }
                None => "El canal esta offline o no hay datos del inicio".to_string(),
            };
            sender::send(&msg, ch, global).await;
            return;
        }
        "!cola" | "!queue" => {
            if !is_owner {
                let ok = { ch.cooldown.lock().await.consume_global("!cola", 30) };
                if !ok {
                    return;
                }
            }
            let q = ch.video_queue.read().await;
            let msg = if q.items.is_empty() {
                "📭 La cola de videos está vacía".to_string()
            } else {
                let lista: Vec<String> = q
                    .items
                    .iter()
                    .enumerate()
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
            sender::send(&msg, ch, global).await;
            return;
        }
        "!seguidores" | "!followers" => {
            if !is_owner {
                let ok = { ch.cooldown.lock().await.consume_global("!seguidores", 60) };
                if !ok {
                    return;
                }
            }
            if !ch.followers_known.load(Ordering::Relaxed) {
                sender::send(
                    "Kick no proporciona un total de seguidores disponible ahora.",
                    ch,
                    global,
                )
                .await;
                return;
            }
            let actual = ch.followers.load(Ordering::Relaxed);
            let goal = ch.follow_goal.load(Ordering::Relaxed);
            let pct = actual.saturating_mul(100).checked_div(goal).unwrap_or(0);
            sender::send(
                &format!("👥 Seguidores: {actual} / {goal} ({pct}%)"),
                ch,
                global,
            )
            .await;
            return;
        }
        _ => {}
    }

    // ── Entretenimiento ───────────────────────────────────────────────────────
    use rand::seq::SliceRandom;
    use rand::Rng;

    if cmd == "!dado" {
        if !is_owner {
            let ok = { ch.cooldown.lock().await.consume_user(username, "!dado", 15) };
            if !ok {
                return;
            }
        }
        let n: u8 = rand::thread_rng().gen_range(1..=100);
        sender::send(&format!("🎲 {username} sacó un {n}!"), ch, global).await;
        return;
    }

    if cmd == "!8ball" || cmd.starts_with("!8ball ") {
        if !is_owner {
            let ok = {
                ch.cooldown
                    .lock()
                    .await
                    .consume_user(username, "!8ball", 10)
            };
            if !ok {
                return;
            }
        }
        const RESP: &[&str] = &[
            "Sí, definitivamente 🟢",
            "Es cierto 🟢",
            "Sin duda 🟢",
            "Por supuesto 🟢",
            "Probablemente sí 🟡",
            "Perspectivas favorables 🟡",
            "No lo sé, pregunta más tarde 🔵",
            "Mejor no te digo ahora 🔵",
            "No cuentes con ello 🔴",
            "Mi respuesta es no 🔴",
            "Muy dudoso 🔴",
        ];
        let resp = RESP.choose(&mut rand::thread_rng()).unwrap_or(&"Quizás");
        sender::send(&format!("🎱 {resp}"), ch, global).await;
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
                let mut s = ch.sorteo.lock().await;
                s.open = true;
                s.participants.clear();
                drop(s);
                sender::send(
                    "🎟️ ¡El sorteo está abierto! Escribe !participar para unirte",
                    ch,
                    global,
                )
                .await;
            }
            "cerrar" | "close" | "stop" if is_owner => {
                let mut s = ch.sorteo.lock().await;
                s.open = false;
                let count = s.participants.len();
                drop(s);
                sender::send(
                    &format!("🔒 Sorteo cerrado. {count} participantes"),
                    ch,
                    global,
                )
                .await;
            }
            "ganador" | "winner" if is_owner => {
                let mut s = ch.sorteo.lock().await;
                s.open = false;
                let winner = s.participants.choose(&mut rand::thread_rng()).cloned();
                drop(s);
                match winner {
                    Some(w) => {
                        sender::send(&format!("🏆 ¡El ganador es @{w}! 🎉"), ch, global).await
                    }
                    None => sender::send("😅 No hay participantes", ch, global).await,
                }
            }
            "" => {
                let mut s = ch.sorteo.lock().await;
                if !s.open
                    || s.participants
                        .iter()
                        .any(|p| p.eq_ignore_ascii_case(username))
                {
                    return;
                }
                s.participants.push(username.to_string());
                let count = s.participants.len();
                drop(s);
                sender::send(
                    &format!("✅ @{username} se unió al sorteo! ({count})"),
                    ch,
                    global,
                )
                .await;
            }
            _ => {}
        }
        return;
    }

    // ── Gestión de cola ───────────────────────────────────────────────────────
    if cmd == "!quitarme" || cmd == "!removeme" {
        let changed = queue_edit(ch, global, |q| {
            let Some(pos) = q
                .items
                .iter()
                .position(|v| v.user.eq_ignore_ascii_case(username))
            else {
                return false;
            };
            q.remove(pos);
            true
        })
        .await;
        if changed {
            sender::send(
                &format!("@{username} retiro su video de la cola"),
                ch,
                global,
            )
            .await;
        }
        return;
    }

    if cmd == "!misongs" || cmd == "!miscanciones" {
        if !is_owner {
            let ok = {
                ch.cooldown
                    .lock()
                    .await
                    .consume_user(username, "!misongs", 15)
            };
            if !ok {
                return;
            }
        }
        let q = ch.video_queue.read().await;
        let mis: Vec<String> = q
            .items
            .iter()
            .enumerate()
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
    if let Some(url) = command_args(content, "!play").filter(|url| !url.is_empty()) {
        if !is_owner {
            let ok = { ch.cooldown.lock().await.consume_user(username, "!play", 30) };
            if !ok {
                return;
            }
        }
        play(url.trim().to_string(), username.to_string(), ch, global).await;
        return;
    }

    // ── TTS ───────────────────────────────────────────────────────────────────
    let Some((cmd_word, rest)) = content.split_once(' ') else {
        return;
    };
    let text = rest.trim();
    if text.is_empty() {
        return;
    }
    let cmd_low = cmd_word.to_lowercase();
    let camila = cmd_low == "!s" || cmd_low == "!dai";
    let is_tts = camila
        || cmd_low
            .strip_prefix('!')
            .is_some_and(tts::edge_tts::is_valid_voice);
    if is_tts {
        if !is_owner {
            let ok = { ch.cooldown.lock().await.consume_user(username, "!dai", 15) };
            if !ok {
                return;
            }
        }
        let voice = if camila {
            "camila".into()
        } else {
            cmd_low.trim_start_matches('!').to_string()
        };
        tts::enqueue(ch, text, &voice);
    }
}

/// Devuelve el texto tras `name` si el mensaje es ese comando (sin distinguir mayúsculas).
fn command_args<'a>(content: &'a str, name: &str) -> Option<&'a str> {
    let head = content.get(..name.len())?;
    if !head.eq_ignore_ascii_case(name) {
        return None;
    }
    let rest = &content[name.len()..];
    if rest.is_empty() {
        Some("")
    } else if rest.starts_with(char::is_whitespace) {
        Some(rest.trim())
    } else {
        None
    }
}

async fn info(ch: &Arc<ChannelState>, global: &Arc<AppState>, key: &str, label: &str) {
    let text = {
        let c = ch.commands.read().await;
        match key {
            "discord" => c.discord.clone(),
            "redes" => c.redes.clone(),
            "pc" => c.pc.clone(),
            _ => c.horario.clone(),
        }
    };
    let msg = if text.is_empty() {
        format!("ℹ️ El streamer aún no configuró !{key}")
    } else {
        format!("{label} → {text}")
    };
    sender::send(&msg, ch, global).await;
}

const SET_USAGE: &str =
    "⚙️ Uso: !set discord|redes|pc|horario <texto> (vacío para borrar) · !set meta <número>";

/// `!set <campo> <valor>`: el streamer configura sus comandos desde el chat.
async fn configure(args: &str, ch: &Arc<ChannelState>, global: &Arc<AppState>) {
    let (field, value) = args
        .split_once(char::is_whitespace)
        .map(|(f, v)| (f, v.trim()))
        .unwrap_or((args, ""));
    let field = match field.to_lowercase().as_str() {
        "discord" => "discord",
        "redes" | "rrss" => "redes",
        "pc" | "setup" | "specs" => "pc",
        "horario" | "schedule" => "horario",
        "meta" | "goal" => "meta",
        _ => {
            sender::send(SET_USAGE, ch, global).await;
            return;
        }
    };
    if field == "meta" {
        let Some(goal) = value
            .parse::<u64>()
            .ok()
            .filter(|g| (1..=100_000_000).contains(g))
        else {
            sender::send(SET_USAGE, ch, global).await;
            return;
        };
        if let Err(e) = crate::db::update_follow_goal(&global.db, &ch.slug, goal as i64).await {
            tracing::error!("[Set][{}] {e}", ch.slug);
            sender::send("No se pudo guardar. Intenta más tarde.", ch, global).await;
            return;
        }
        ch.follow_goal
            .store(goal, std::sync::atomic::Ordering::Relaxed);
        sender::send(&format!("✅ Meta de seguidores: {goal}"), ch, global).await;
        return;
    }
    let value = trunc(value, 400);
    if let Err(e) = crate::db::update_command_text(&global.db, &ch.slug, field, value).await {
        tracing::error!("[Set][{}] {e}", ch.slug);
        sender::send("No se pudo guardar. Intenta más tarde.", ch, global).await;
        return;
    }
    {
        let mut c = ch.commands.write().await;
        let slot = match field {
            "discord" => &mut c.discord,
            "redes" => &mut c.redes,
            "pc" => &mut c.pc,
            _ => &mut c.horario,
        };
        *slot = value.to_string();
    }
    let msg = if value.is_empty() {
        format!("🗑️ !{field} borrado")
    } else {
        format!("✅ !{field} actualizado")
    };
    sender::send(&msg, ch, global).await;
}

pub async fn play(url: String, username: String, ch: &Arc<ChannelState>, global: &Arc<AppState>) {
    if ch.cancel.is_cancelled() || url.len() > 2048 || !valid_media_url(&url) {
        return;
    }

    if is_direct_video(&url) {
        let title = url
            .split('/')
            .next_back()
            .and_then(|s| s.split('?').next())
            .unwrap_or("Video")
            .to_string();
        let msg = format!("▶ @{username} agregó «{}» a la cola", trunc(&title, 60));
        if enqueue(
            VideoItem {
                id: uuid::Uuid::new_v4().to_string(),
                video_id: None,
                url: Some(url),
                title,
                user: username,
            },
            ch,
            global,
        )
        .await
        {
            sender::send(&msg, ch, global).await;
        }
        return;
    }

    if url.contains("list=") {
        let list_id = url
            .split("list=")
            .nth(1)
            .unwrap_or("")
            .split('&')
            .next()
            .unwrap_or("");
        if !list_id.starts_with("RD") {
            if let Some((vid, title)) = first_from_playlist(&global.http, &url).await {
                let msg = format!("▶ @{username} agregó «{}» a la cola", trunc(&title, 60));
                if enqueue(
                    VideoItem {
                        id: uuid::Uuid::new_v4().to_string(),
                        video_id: Some(vid),
                        url: None,
                        title,
                        user: username,
                    },
                    ch,
                    global,
                )
                .await
                {
                    sender::send(&msg, ch, global).await;
                }
                return;
            }
        }
    }

    if let Some(vid) = yt_id(&url) {
        let title = yt_title(
            &global.http,
            &format!("https://www.youtube.com/watch?v={vid}"),
        )
        .await;
        let msg = format!("▶ @{username} agregó «{}» a la cola", trunc(&title, 60));
        if enqueue(
            VideoItem {
                id: uuid::Uuid::new_v4().to_string(),
                video_id: Some(vid),
                url: None,
                title,
                user: username,
            },
            ch,
            global,
        )
        .await
        {
            sender::send(&msg, ch, global).await;
        }
    } else {
        warn!("[PLAY] URL inválida: {url}");
        sender::send(
            &format!("@{username} no pude procesar esa URL."),
            ch,
            global,
        )
        .await;
    }
}

async fn enqueue(item: VideoItem, ch: &Arc<ChannelState>, global: &Arc<AppState>) -> bool {
    queue_edit(ch, global, |q| {
        if q.items.len() >= 100 {
            return false;
        }
        q.push(item);
        true
    })
    .await
}

async fn queue_edit<F>(ch: &Arc<ChannelState>, global: &Arc<AppState>, edit: F) -> bool
where
    F: FnOnce(&mut crate::queue::VideoQueue) -> bool,
{
    match crate::queue::change(ch, global, edit).await {
        Ok(changed) => changed,
        Err(e) => {
            tracing::error!("[Queue][{}] {e}", ch.slug);
            sender::send("No se pudo guardar la cola. Intenta mas tarde.", ch, global).await;
            false
        }
    }
}
async fn visibility(ch: &Arc<ChannelState>, global: &Arc<AppState>, visible: bool) {
    if let Err(e) = crate::server::set_visibility(ch, global, visible).await {
        tracing::error!("[Visibility][{}] {e}", ch.slug);
    }
}

fn valid_media_url(value: &str) -> bool {
    let Ok(url) = url::Url::parse(value) else {
        return false;
    };
    if url.scheme() != "https" || !url.username().is_empty() || url.password().is_some() {
        return false;
    }
    let Some(host) = url.host_str() else {
        return false;
    };
    // Media must not target local addresses of the viewer/OBS machine.
    if host == "localhost"
        || host.ends_with(".localhost")
        || host.ends_with(".local")
        || !host.contains('.')
    {
        return false;
    }
    if let Ok(ip) = host.trim_matches(['[', ']']).parse::<std::net::IpAddr>() {
        return match ip {
            std::net::IpAddr::V4(v) => {
                !v.is_private()
                    && !v.is_loopback()
                    && !v.is_link_local()
                    && !v.is_unspecified()
                    && !v.is_broadcast()
                    && !v.is_multicast()
            }
            std::net::IpAddr::V6(_) => false,
        };
    }
    true
}
fn is_direct_video(value: &str) -> bool {
    if !valid_media_url(value) {
        return false;
    }
    let Ok(url) = url::Url::parse(value) else {
        return false;
    };
    matches!(
        std::path::Path::new(url.path())
            .extension()
            .and_then(|x| x.to_str()),
        Some("mp4" | "webm" | "mov" | "m4v")
    )
}
fn yt_id(value: &str) -> Option<String> {
    let url = url::Url::parse(value).ok()?;
    if url.scheme() != "https" {
        return None;
    }
    let id = match url.host_str()? {
        "youtu.be" => url.path().trim_start_matches('/').to_owned(),
        "youtube.com" | "www.youtube.com" | "m.youtube.com" => {
            if url.path() == "/watch" {
                url.query_pairs().find(|(k, _)| k == "v")?.1.into_owned()
            } else {
                let path = url.path();
                path.strip_prefix("/embed/")
                    .or_else(|| path.strip_prefix("/shorts/"))?
                    .to_owned()
            }
        }
        _ => return None,
    };
    (id.len() == 11
        && id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-'))
    .then_some(id)
}

async fn first_from_playlist(http: &reqwest::Client, url: &str) -> Option<(String, String)> {
    let parsed = url::Url::parse(url).ok()?;
    if !matches!(
        parsed.host_str()?,
        "youtube.com" | "www.youtube.com" | "m.youtube.com"
    ) {
        return None;
    }
    let list_id = parsed
        .query_pairs()
        .find(|(k, _)| k == "list")?
        .1
        .into_owned();
    if !list_id
        .bytes()
        .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
    {
        return None;
    }
    let key = std::env::var("YOUTUBE_API_KEY").ok()?;
    if list_id.starts_with("RD") {
        return None;
    }
    let api = format!(
        "https://www.googleapis.com/youtube/v3/playlistItems?part=snippet&maxResults=1&playlistId={list_id}&key={key}"
    );
    let json: serde_json::Value = http.get(&api).send().await.ok()?.json().await.ok()?;
    let item = json["items"].as_array()?.first()?;
    let vid = item["snippet"]["resourceId"]["videoId"]
        .as_str()?
        .to_string();
    let title = item["snippet"]["title"]
        .as_str()
        .unwrap_or("Video")
        .to_string();
    Some((vid, title))
}

fn trunc(s: &str, max_chars: usize) -> &str {
    match s.char_indices().nth(max_chars) {
        Some((i, _)) => &s[..i],
        None => s,
    }
}

async fn yt_title(http: &reqwest::Client, url: &str) -> String {
    let enc: String = url
        .bytes()
        .flat_map(|b| {
            if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.' | b'~') {
                vec![b as char]
            } else {
                format!("%{b:02X}").chars().collect()
            }
        })
        .collect();
    let api = format!("https://noembed.com/embed?url={enc}");
    let Ok(resp) = http.get(&api).send().await else {
        return "Video de YouTube".to_string();
    };
    let Ok(json) = resp.json::<serde_json::Value>().await else {
        return "Video de YouTube".to_string();
    };
    json["title"]
        .as_str()
        .unwrap_or("Video de YouTube")
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn youtube_parsing_rejects_spoofed_hosts_and_ids() {
        assert_eq!(
            yt_id("https://youtu.be/dQw4w9WgXcQ"),
            Some("dQw4w9WgXcQ".into())
        );
        assert_eq!(
            yt_id("https://www.youtube.com/watch?v=dQw4w9WgXcQ&list=x"),
            Some("dQw4w9WgXcQ".into())
        );
        assert!(yt_id("https://evil.example/?v=dQw4w9WgXcQ").is_none());
        assert!(yt_id("https://youtube.com.evil.example/watch?v=dQw4w9WgXcQ").is_none());
        assert!(yt_id("https://youtu.be/short").is_none());
    }
    #[test]
    fn direct_media_requires_public_https() {
        assert!(is_direct_video("https://cdn.example.com/video.mp4?x=1"));
        assert!(!is_direct_video("http://cdn.example.com/video.mp4"));
        assert!(!is_direct_video("https://127.0.0.1/video.mp4"));
        assert!(!is_direct_video("https://10.0.0.1/video.mp4"));
        assert!(!is_direct_video(
            "https://user:pass@cdn.example.com/video.mp4"
        ));
    }
    #[test]
    fn command_args_ignores_case_and_requires_separator() {
        assert_eq!(
            command_args("!Play https://x.y", "!play"),
            Some("https://x.y")
        );
        assert_eq!(command_args("!play", "!play"), Some(""));
        assert_eq!(command_args("!playlist", "!play"), None);
        assert_eq!(
            command_args("!SET discord https://d.gg/A", "!set"),
            Some("discord https://d.gg/A")
        );
        assert_eq!(command_args("!p", "!play"), None);
    }
    #[test]
    fn unicode_truncation_never_splits_code_points() {
        assert_eq!(trunc("aé😀b", 3), "aé😀");
    }
}
