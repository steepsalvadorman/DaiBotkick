use crate::{
    kick::sender,
    queue::VideoItem,
    state::{AppState, ChannelState},
    tts,
};
use std::sync::Arc;
use tracing::warn;
mod search;
mod tiktok;

/// Datos del mensaje de chat que reciben todos los grupos de comandos.
#[derive(Clone, Copy)]
struct Ctx<'a> {
    username: &'a str,
    content: &'a str,
    /// `content` en minúsculas.
    cmd: &'a str,
    is_owner: bool,
    ch: &'a Arc<ChannelState>,
    global: &'a Arc<AppState>,
}

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
    if crate::bingo::handle(username, cmd.trim(), is_owner, ch, global).await {
        return;
    }
    let ctx = Ctx {
        username,
        content,
        cmd: &cmd,
        is_owner,
        ch,
        global,
    };
    let handled = owner_commands(&ctx).await
        || info_commands(&ctx).await
        || dynamic_commands(&ctx).await
        || entertainment_commands(&ctx).await
        || queue_commands(&ctx).await
        || play_command(&ctx).await;
    if !handled {
        tts_command(&ctx).await;
    }
}

async fn owner_commands(c: &Ctx<'_>) -> bool {
    let Ctx {
        content,
        cmd,
        is_owner,
        ch,
        global,
        ..
    } = *c;
    // ── Comandos solo del owner ───────────────────────────────────────────────
    if is_owner {
        if let Some(args) = command_args(content, "!set") {
            configure(args, ch, global).await;
            return true;
        }
        match cmd {
            "!von" => {
                visibility(ch, global, true).await;
                return true;
            }
            "!voff" => {
                visibility(ch, global, false).await;
                return true;
            }
            "!vstop" => {
                queue_edit(ch, global, |q| {
                    q.clear();
                    true
                })
                .await;
                return true;
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
                return true;
            }
            _ => {}
        }
    }
    false
}

async fn info_commands(c: &Ctx<'_>) -> bool {
    let Ctx {
        cmd,
        is_owner,
        ch,
        global,
        ..
    } = *c;
    // ── Comandos informativos (cooldown 20s) ──────────────────────────────────
    macro_rules! global_cmd {
        ($key:expr, $body:expr) => {{
            if global_ready(ch, is_owner, $key, INFO_COOLDOWN_SECS).await {
                $body
            }
            return true;
        }};
    }

    match cmd {
        "!voces" => global_cmd!("!voces", {
            sender::send(
                &tts::voice_list(!global.config.fish_audio_api_key.trim().is_empty()),
                ch,
                global,
            )
            .await;
        }),
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
                "📋 Comandos: !play [url o canción] · !voces · !s/!dalia/!jorge/!alex/!narrador/!epico/!comedia/!jacinta [texto TTS] · !quitarme · !misongs · !dado · !carton · !bingo · !8ball [pregunta] · !sorteo · !uptime · !cola · !discord · !redes · !pc · !horario",
                ch, global,
            ).await;
        }),
        _ => {}
    }
    false
}

async fn dynamic_commands(c: &Ctx<'_>) -> bool {
    let Ctx {
        cmd,
        is_owner,
        ch,
        global,
        ..
    } = *c;
    // ── Comandos dinámicos ────────────────────────────────────────────────────
    use std::sync::atomic::Ordering;

    match cmd {
        "!uptime" => {
            if !global_ready(ch, is_owner, "!uptime", 30).await {
                return true;
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
            return true;
        }
        "!cola" | "!queue" => {
            if !global_ready(ch, is_owner, "!cola", 30).await {
                return true;
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
            return true;
        }
        "!seguidores" | "!followers" => {
            if !global_ready(ch, is_owner, "!seguidores", 60).await {
                return true;
            }
            if !ch.followers_known.load(Ordering::Relaxed) {
                sender::send(
                    "Kick no proporciona un total de seguidores disponible ahora.",
                    ch,
                    global,
                )
                .await;
                return true;
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
            return true;
        }
        _ => {}
    }
    false
}

async fn entertainment_commands(c: &Ctx<'_>) -> bool {
    let Ctx {
        username,
        cmd,
        is_owner,
        ch,
        global,
        ..
    } = *c;
    // ── Entretenimiento ───────────────────────────────────────────────────────
    use rand::seq::SliceRandom;
    use rand::Rng;

    if cmd == "!dado" {
        if !user_ready(ch, is_owner, username, "!dado", 15).await {
            return true;
        }
        let n: u8 = rand::thread_rng().gen_range(1..=100);
        sender::send(&format!("🎲 {username} sacó un {n}!"), ch, global).await;
        return true;
    }

    if cmd == "!8ball" || cmd.starts_with("!8ball ") {
        if !user_ready(ch, is_owner, username, "!8ball", 10).await {
            return true;
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
        return true;
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
                    return true;
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
        return true;
    }
    false
}

async fn queue_commands(c: &Ctx<'_>) -> bool {
    let Ctx {
        username,
        cmd,
        is_owner,
        ch,
        global,
        ..
    } = *c;
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
        return true;
    }

    if cmd == "!misongs" || cmd == "!miscanciones" {
        if !user_ready(ch, is_owner, username, "!misongs", 15).await {
            return true;
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
        return true;
    }
    false
}

async fn play_command(c: &Ctx<'_>) -> bool {
    let Ctx {
        username,
        content,
        is_owner,
        ch,
        global,
        ..
    } = *c;
    // ── !play ─────────────────────────────────────────────────────────────────
    if let Some(url) = command_args(content, "!play") {
        if is_play_blocked(username) {
            return true;
        }
        if url.is_empty() {
            sender::send(
                "Usa !play seguido de un enlace o del nombre y artista de la canción.",
                ch,
                global,
            )
            .await;
            return true;
        }
        if !user_ready(ch, is_owner, username, "!play", 30).await {
            return true;
        }
        play(url.trim().to_string(), username.to_string(), ch, global).await;
        return true;
    }
    false
}

async fn tts_command(c: &Ctx<'_>) {
    let Ctx {
        username,
        content,
        is_owner,
        ch,
        global,
        ..
    } = *c;
    // ── TTS ───────────────────────────────────────────────────────────────────
    let Some((cmd_word, rest)) = content.split_once(' ') else {
        return;
    };
    let text = rest.trim();
    if text.is_empty() {
        return;
    }
    let cmd_low = cmd_word.to_lowercase();
    let default_voice = cmd_low == "!s";
    let is_tts = default_voice || cmd_low.strip_prefix('!').is_some_and(tts::is_valid_voice);
    if is_tts {
        if !user_ready(ch, is_owner, username, "!camila", 15).await {
            return;
        }
        let voice = if default_voice {
            tts::default_chat_voice().into()
        } else {
            cmd_low.trim_start_matches('!').to_string()
        };
        if !tts::enqueue_chat(ch, username, text, &voice) {
            sender::send(
                &format!("No se pudo agregar tu mensaje de voz: usa hasta {} caracteres y vuelve a intentar si la cola está llena.", tts::KICK_MESSAGE_MAX),
                ch, global,
            ).await;
        }
    }
}

const INFO_COOLDOWN_SECS: u64 = 20;
const MAX_QUEUE_ITEMS: usize = 100;
const MAX_URL_LEN: usize = 2048;
const TITLE_PREVIEW_CHARS: usize = 60;

/// El owner no tiene cooldown; el resto consume el cooldown global del comando.
async fn global_ready(ch: &ChannelState, is_owner: bool, key: &str, secs: u64) -> bool {
    is_owner || ch.cooldown.lock().await.consume_global(key, secs)
}

/// El owner no tiene cooldown; el resto consume el cooldown por usuario del comando.
async fn user_ready(ch: &ChannelState, is_owner: bool, user: &str, key: &str, secs: u64) -> bool {
    is_owner || ch.cooldown.lock().await.consume_user(user, key, secs)
}

/// Usuarios que no pueden pedir canciones con !play; sus peticiones se ignoran en silencio.
const PLAY_BLOCKED_USERS: &[&str] = &["theblastv2"];

fn is_play_blocked(username: &str) -> bool {
    let name = username.trim();
    PLAY_BLOCKED_USERS
        .iter()
        .any(|blocked| name.eq_ignore_ascii_case(blocked))
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
    if ch.cancel.is_cancelled() {
        return;
    }
    let url = with_scheme(url);
    if !url.contains("://") {
        let found = tokio::select! {
            _ = ch.cancel.cancelled() => return,
            result = search::youtube(url.trim(), global) => result,
        };
        match found {
            Ok((id, title)) => {
                let message = format!(
                    "▶ @{username} encontré «{}» y la agregué a la cola.",
                    trunc(&title, TITLE_PREVIEW_CHARS)
                );
                let item = VideoItem {
                    video_id: Some(id),
                    ..new_item(username, title)
                };
                enqueue_announced(item, &message, ch, global).await;
            }
            Err(message) => sender::send(message, ch, global).await,
        }
        return;
    }
    if url.len() > MAX_URL_LEN || !valid_media_url(&url) {
        sender::send(
            "Enlace inválido. Usa una URL HTTPS o escribe el nombre y artista de la canción.",
            ch,
            global,
        )
        .await;
        return;
    }

    if tiktok::is_link(&url) {
        let resolved = tokio::select! {
            _ = ch.cancel.cancelled() => return,
            result = tiktok::resolve(&url) => result,
        };
        match resolved {
            Ok((tiktok_id, title)) => {
                let item = VideoItem {
                    tiktok_id: Some(tiktok_id),
                    ..new_item(username.clone(), title)
                };
                add_announced(item, &username, ch, global).await;
            }
            Err(message) => sender::send(&format!("@{username} {message}"), ch, global).await,
        }
        return;
    }

    if is_direct_video(&url) {
        let title = url
            .split('/')
            .next_back()
            .and_then(|s| s.split('?').next())
            .unwrap_or("Video")
            .to_string();
        let item = VideoItem {
            url: Some(url),
            ..new_item(username.clone(), title)
        };
        add_announced(item, &username, ch, global).await;
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
                let item = VideoItem {
                    video_id: Some(vid),
                    ..new_item(username.clone(), title)
                };
                add_announced(item, &username, ch, global).await;
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
        let item = VideoItem {
            video_id: Some(vid),
            ..new_item(username.clone(), title)
        };
        add_announced(item, &username, ch, global).await;
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

/// Item de cola sin fuente; quien lo usa rellena `video_id`, `tiktok_id` o `url`.
fn new_item(user: String, title: String) -> VideoItem {
    VideoItem {
        id: uuid::Uuid::new_v4().to_string(),
        video_id: None,
        tiktok_id: None,
        url: None,
        title,
        user,
    }
}

/// Encola `item` y, si entró, anuncia en el chat que `username` lo agregó.
async fn add_announced(
    item: VideoItem,
    username: &str,
    ch: &Arc<ChannelState>,
    global: &Arc<AppState>,
) {
    let message = format!(
        "▶ @{username} agregó «{}» a la cola",
        trunc(&item.title, TITLE_PREVIEW_CHARS)
    );
    enqueue_announced(item, &message, ch, global).await;
}

async fn enqueue_announced(
    item: VideoItem,
    message: &str,
    ch: &Arc<ChannelState>,
    global: &Arc<AppState>,
) {
    if enqueue(item, ch, global).await {
        sender::send(message, ch, global).await;
    }
}

async fn enqueue(item: VideoItem, ch: &Arc<ChannelState>, global: &Arc<AppState>) -> bool {
    if ch.cancel.is_cancelled() {
        return false;
    }
    if ch.video_queue.read().await.items.len() >= MAX_QUEUE_ITEMS {
        sender::send(
            &format!("La cola está llena ({MAX_QUEUE_ITEMS} solicitudes). Espera a que termine una canción."),
            ch,
            global,
        )
        .await;
        return false;
    }
    queue_edit(ch, global, |q| {
        if q.items.len() >= MAX_QUEUE_ITEMS {
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

/// Acepta enlaces pegados sin esquema (www.youtube.com/..., vm.tiktok.com/...) y el ID
/// numérico de un video de TikTok solo: el chat de Kick rechaza mensajes con muchos símbolos.
fn with_scheme(url: String) -> String {
    if (15..=25).contains(&url.len()) && url.bytes().all(|b| b.is_ascii_digit()) {
        return format!("https://www.tiktok.com/video/{url}");
    }
    const HOSTS: [&str; 7] = [
        "www.",
        "youtube.com",
        "youtu.be",
        "m.youtube.com",
        "tiktok.com",
        "vm.tiktok.com",
        "vt.tiktok.com",
    ];
    if !url.contains("://") && !url.contains(' ') && HOSTS.iter().any(|h| url.starts_with(h)) {
        format!("https://{url}")
    } else {
        url
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
    fn pasted_links_without_scheme_become_https() {
        assert_eq!(
            with_scheme("vm.tiktok.com/ZMhvqjqjq/".into()),
            "https://vm.tiktok.com/ZMhvqjqjq/"
        );
        assert_eq!(
            with_scheme("www.youtube.com/watch?v=dQw4w9WgXcQ".into()),
            "https://www.youtube.com/watch?v=dQw4w9WgXcQ"
        );
        assert_eq!(
            with_scheme("7671109760938265874".into()),
            "https://www.tiktok.com/video/7671109760938265874"
        );
        assert_eq!(with_scheme("1999".into()), "1999");
        assert_eq!(
            with_scheme("cervecita flor pileña".into()),
            "cervecita flor pileña"
        );
        assert_eq!(
            with_scheme("tiktok.com es genial".into()),
            "tiktok.com es genial"
        );
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
    fn blocked_users_cannot_play() {
        for name in ["TheBlastv2", "theblastv2", " THEBLASTV2 "] {
            assert!(is_play_blocked(name));
        }
        assert!(!is_play_blocked("viewer"));
        assert!(!is_play_blocked("theblastv22"));
    }
    #[test]
    fn unicode_truncation_never_splits_code_points() {
        assert_eq!(trunc("aé😀b", 3), "aé😀");
    }
}
