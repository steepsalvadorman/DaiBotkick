pub mod api;
pub mod sender;

use crate::{commands, db, server::ns_emit, state::{AppState, ChannelState}, tts};
use futures_util::{SinkExt, StreamExt};
use serde::Deserialize;
use serde_json::json;
use std::sync::{atomic::Ordering, Arc};
use tokio::time::{sleep, Duration};
use tokio_tungstenite::{connect_async, tungstenite::Message};
use tracing::{debug, error, info, warn};

const PUSHER_KEY_PRIMARY: &str = "32cbd69e4b950bf97679";
const PUSHER_KEY_ALT:     &str = "eb1d5f283081a78b932c";
const PUSHER_ENDPOINTS: &[(&str, &str)] = &[
    ("ws-us2.pusher.com",  PUSHER_KEY_PRIMARY),
    ("ws-eu.pusher.com",   PUSHER_KEY_ALT),
    ("ws-mt1.pusher.com",  PUSHER_KEY_ALT),
    ("ws-ap1.pusher.com",  PUSHER_KEY_ALT),
    ("ws-us3.pusher.com",  PUSHER_KEY_ALT),
    ("ws-sa1.pusher.com",  PUSHER_KEY_ALT),
];

#[derive(Deserialize)]
struct PusherEvent {
    event:   String,
    data:    Option<serde_json::Value>,
    #[allow(dead_code)]
    channel: Option<String>,
}

#[derive(Deserialize)]
struct KickChatMsg {
    content: String,
    sender:  KickSender,
}

#[derive(Deserialize)]
struct KickSender { username: String }

fn unix_now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

// ─── Token refresh ────────────────────────────────────────────────────────────

pub async fn refresh_access_token(ch: &Arc<ChannelState>, global: &Arc<AppState>) -> bool {
    let client_id     = &global.config.client_id;
    let client_secret = &global.config.client_secret;
    let refresh_tok   = ch.refresh_token_val.read().await.clone();

    if client_id.is_empty() || client_secret.is_empty() || refresh_tok.is_empty() {
        warn!("[OAuth][{}] Faltan credenciales para renovar token", ch.slug);
        return false;
    }

    let resp = global.http
        .post("https://id.kick.com/oauth/token")
        .form(&[
            ("grant_type",    "refresh_token"),
            ("refresh_token", refresh_tok.as_str()),
            ("client_id",     client_id.as_str()),
            ("client_secret", client_secret.as_str()),
        ])
        .send()
        .await;

    let r = match resp {
        Ok(r) => r,
        Err(e) => { error!("[OAuth][{}] Red: {e}", ch.slug); return false; }
    };

    if !r.status().is_success() {
        error!("[OAuth][{}] Refresh falló: {}", ch.slug, r.status());
        return false;
    }

    let data: serde_json::Value = match r.json().await {
        Ok(d)  => d,
        Err(e) => { error!("[OAuth][{}] JSON inválido: {e}", ch.slug); return false; }
    };

    let Some(new_access) = data["access_token"].as_str() else {
        error!("[OAuth][{}] Sin access_token: {data}", ch.slug);
        return false;
    };
    let new_refresh  = data["refresh_token"].as_str().unwrap_or(&refresh_tok);
    let expires_in   = data["expires_in"].as_u64().unwrap_or(7200);
    let new_expires  = (unix_now() + expires_in) as i64;

    *ch.access_token.write().await      = new_access.to_string();
    *ch.refresh_token_val.write().await = new_refresh.to_string();
    db::update_tokens(&global.db, &ch.slug, new_access, new_refresh, new_expires).await;

    info!("[OAuth][{}] Token renovado, expira en {expires_in}s", ch.slug);
    true
}

pub async fn token_refresh_loop(ch: Arc<ChannelState>, global: Arc<AppState>, initial_expires: u64) {
    let mut expires = initial_expires;
    loop {
        let now = unix_now();
        let sleep_secs = if expires > now + 610 { expires - now - 600 } else { 60 };
        sleep(Duration::from_secs(sleep_secs)).await;

        info!("[OAuth][{}] Renovando token proactivamente...", ch.slug);
        if refresh_access_token(&ch, &global).await {
            expires = unix_now() + 7200;
        }
    }
}

// ─── Punto de entrada por canal ───────────────────────────────────────────────

pub async fn run_channel(ch: Arc<ChannelState>, global: Arc<AppState>, token_expires: u64) {

    // Auto-renovación de token
    let ch3 = ch.clone(); let g3 = global.clone();
    tokio::spawn(async move { token_refresh_loop(ch3, g3, token_expires).await; });

    // EventSub subscriptions (3s delay para que el servidor esté listo)
    let ch4 = ch.clone(); let g4 = global.clone();
    tokio::spawn(async move {
        sleep(Duration::from_secs(5)).await;
        let broadcaster_id = loop {
            let id = *ch4.channel_id.read().await;
            if let Some(id) = id { break id; }
            sleep(Duration::from_secs(2)).await;
        };
        subscribe_kick_events(&ch4, &g4, broadcaster_id).await;
    });

    loop {
        match connect_once(&ch, &global).await {
            Ok(_)  => warn!("[Kick][{}] Conexión cerrada — reconectando en 5s…", ch.slug),
            Err(e) => error!("[Kick][{}] {e} — reconectando en 5s…", ch.slug),
        }
        sleep(Duration::from_secs(5)).await;
    }
}

// ─── Suscripciones EventSub ───────────────────────────────────────────────────

async fn subscribe_kick_events(ch: &Arc<ChannelState>, global: &Arc<AppState>, broadcaster_user_id: u64) {
    let token = ch.access_token.read().await.clone();
    let body = json!({
        "events": [
            { "name": "chat.message.sent",          "version": 1 },
            { "name": "channel.followed",            "version": 1 },
            { "name": "channel.subscription.new",    "version": 1 },
            { "name": "channel.subscription.renewed","version": 1 },
            { "name": "channel.subscription.gifts",  "version": 1 }
        ],
        "method": "webhook",
        "broadcaster_user_id": broadcaster_user_id
    });

    match global.http
        .post("https://api.kick.com/public/v1/events/subscriptions")
        .header("Authorization", format!("Bearer {token}"))
        .json(&body)
        .send()
        .await
    {
        Ok(r) => {
            let status = r.status();
            let text   = r.text().await.unwrap_or_default();
            if status.is_success() {
                info!("[EventSub][{}] Suscripciones creadas", ch.slug);
            } else {
                warn!("[EventSub][{}] {status}: {text:.200}", ch.slug);
            }
        }
        Err(e) => warn!("[EventSub][{}] Error: {e}", ch.slug),
    }
}

// ─── Conexión Pusher ──────────────────────────────────────────────────────────

async fn connect_once(ch: &Arc<ChannelState>, global: &Arc<AppState>) -> Result<(), String> {
    let token = ch.access_token.read().await.clone();
    let info = match api::get_channel_info(&global.http, &ch.slug, &token).await {
        Some(i) => i,
        None => {
            warn!("[Kick][{}] Token inválido, renovando...", ch.slug);
            if refresh_access_token(ch, global).await {
                let new_tok = ch.access_token.read().await.clone();
                api::get_channel_info(&global.http, &ch.slug, &new_tok)
                    .await
                    .ok_or_else(|| format!("No se pudo obtener info del canal '{}'", ch.slug))?
            } else {
                return Err(format!("Token expirado para '{}'", ch.slug));
            }
        }
    };

    info!("[Kick] Canal '{}' → channel_id={} chatroom_id={}", info.slug, info.channel_id, info.chatroom_id);
    *ch.channel_id.write().await  = Some(info.channel_id);
    *ch.chatroom_id.write().await = Some(info.chatroom_id);

    // Actualizar IDs en DB
    db::update_channel_ids(
        &global.db,
        &ch.slug,
        info.channel_id  as i64,
        info.chatroom_id as i64,
    ).await;

    // Actualizar el mapa broadcaster_id → slug
    global.user_id_to_slug.insert(info.channel_id, ch.slug.clone());

    // Conectar al Pusher correcto
    let override_host = std::env::var("PUSHER_HOST").ok();
    let endpoints_to_try: Vec<(&str, &str)> = if let Some(ref h) = override_host {
        vec![(h.as_str(), PUSHER_KEY_PRIMARY)]
    } else {
        PUSHER_ENDPOINTS.to_vec()
    };

    let mut found = None;
    for (pusher_host, pusher_key) in &endpoints_to_try {
        let ws_url = format!(
            "wss://{pusher_host}/app/{pusher_key}?protocol=7&client=js&version=8.5.0&flash=false"
        );
        info!("[Kick][{}] Probando WebSocket: {pusher_host}", ch.slug);
        let request = {
            use tokio_tungstenite::tungstenite::client::IntoClientRequest;
            let mut r = ws_url.as_str().into_client_request()
                .map_err(|e| format!("WS request: {e}"))?;
            r.headers_mut().insert(
                tokio_tungstenite::tungstenite::http::header::ORIGIN,
                tokio_tungstenite::tungstenite::http::HeaderValue::from_static("https://kick.com"),
            );
            r
        };
        let ws = match connect_async(request).await {
            Ok((ws, _)) => ws,
            Err(e) => { warn!("  {pusher_host}: {e}"); continue; }
        };
        let (tx, mut rx) = ws.split();
        match wait_for_socket_id(&mut rx).await {
            Some(sid) => {
                info!("  ✓ Conectado a {pusher_host} socket_id={sid}");
                found = Some((tx, rx, sid));
                break;
            }
            None => continue,
        }
    }

    let (mut tx, mut rx, socket_id) = found
        .ok_or_else(|| "Ningún endpoint de Pusher respondió".to_string())?;

    info!("[Kick][{}] Pusher socket_id={socket_id}", ch.slug);

    // Canal privado con auth, fallback a público
    let private_chat = format!("private-chatrooms.{}.v2", info.chatroom_id);
    let public_chat  = format!("chatrooms.{}.v2", info.chatroom_id);
    let token2 = ch.access_token.read().await.clone();

    let chat_channel = match pusher_auth(&global.http, &socket_id, &private_chat, &token2).await {
        Some(auth) => {
            info!("[Kick][{}] Auth Pusher OK — canal privado", ch.slug);
            subscribe(&mut tx, &private_chat, Some(&auth)).await?;
            private_chat
        }
        None => {
            debug!("[Kick][{}] Auth Pusher falló — usando canal público", ch.slug);
            subscribe(&mut tx, &public_chat, None).await?;
            public_chat
        }
    };
    info!("[Kick][{}] Suscrito a {chat_channel}", ch.slug);

    let events_channel = format!("channel.{}", info.slug);
    subscribe(&mut tx, &events_channel, None).await?;
    info!("[Kick][{}] Suscrito a {events_channel}", ch.slug);

    while let Some(msg) = rx.next().await {
        match msg {
            Ok(Message::Text(raw))  => { on_event(&raw, &mut tx, ch, global).await; }
            Ok(Message::Ping(d))    => { tx.send(Message::Pong(d)).await.ok(); }
            Ok(Message::Close(_))   => { warn!("[Kick][{}] Pusher cerró la conexión", ch.slug); break; }
            Err(e)                  => return Err(format!("WS: {e}")),
            _                       => {}
        }
    }
    Ok(())
}

async fn subscribe(
    tx: &mut (impl SinkExt<Message, Error = tokio_tungstenite::tungstenite::Error> + Unpin),
    channel: &str,
    auth: Option<&str>,
) -> Result<(), String> {
    let msg = json!({
        "event": "pusher:subscribe",
        "data":  { "auth": auth.unwrap_or(""), "channel": channel }
    });
    tx.send(Message::Text(msg.to_string())).await.map_err(|e| format!("Subscribe {channel}: {e}"))
}

async fn wait_for_socket_id(
    rx: &mut (impl StreamExt<Item = Result<Message, tokio_tungstenite::tungstenite::Error>> + Unpin),
) -> Option<String> {
    while let Some(msg) = rx.next().await {
        match msg {
            Ok(Message::Text(raw)) => {
                let Ok(ev) = serde_json::from_str::<PusherEvent>(&raw) else { continue };
                match ev.event.as_str() {
                    "pusher:connection_established" => {
                        let data_str = match &ev.data {
                            Some(serde_json::Value::String(s)) => s.clone(),
                            Some(v) => v.to_string(),
                            None    => continue,
                        };
                        let conn: serde_json::Value = serde_json::from_str(&data_str).ok()?;
                        return conn["socket_id"].as_str().map(|s| s.to_string());
                    }
                    "pusher:error" => { error!("[Pusher] Error: {raw}"); return None; }
                    _ => {}
                }
            }
            Ok(Message::Close(_)) => return None,
            Err(e) => { warn!("[Pusher] WS error en handshake: {e}"); return None; }
            _ => {}
        }
    }
    None
}

// ─── Manejador de eventos ─────────────────────────────────────────────────────

async fn on_event(
    raw:    &str,
    tx:     &mut (impl SinkExt<Message, Error = tokio_tungstenite::tungstenite::Error> + Unpin),
    ch:     &Arc<ChannelState>,
    global: &Arc<AppState>,
) {
    let Ok(ev) = serde_json::from_str::<PusherEvent>(raw) else {
        debug!("[Pusher] JSON no parseable: {raw}");
        return;
    };

    match ev.event.as_str() {
        "pusher:ping" => {
            tx.send(Message::Text(json!({"event":"pusher:pong","data":{}}).to_string())).await.ok();
        }
        "App\\Events\\ChatMessageEvent" => {
            let data_str = raw_data_str(&ev.data);
            let Ok(msg) = serde_json::from_str::<KickChatMsg>(&data_str) else { return };
            let username = msg.sender.username.clone();
            let content  = msg.content.trim().to_string();
            ns_emit(global, &ch.slug, "chatMessage", json!({"user": &username, "content": &content}));
            commands::handle(&username, &content, ch, global).await;
        }
        "App\\Events\\FollowersUpdated" => {
            let data = serde_json::from_str::<serde_json::Value>(&raw_data_str(&ev.data)).unwrap_or_default();
            if let Some(count) = data["followersCount"].as_u64().or_else(|| data["followers_count"].as_u64()) {
                ch.followers.store(count, Ordering::Relaxed);
                ns_emit(global, &ch.slug, "followersUpdate", json!({"count": count}));
                info!("[Kick][{}] Seguidores: {count}", ch.slug);
            }
        }
        "App\\Events\\FollowEvent" => {
            let data = serde_json::from_str::<serde_json::Value>(&raw_data_str(&ev.data)).unwrap_or_default();
            let username = data["user_username"].as_str().or_else(|| data["username"].as_str()).unwrap_or("alguien");
            info!("[Kick][{}] Follow: {username}", ch.slug);
            alert_follow(username, ch, global).await;
        }
        "App\\Events\\SubscriptionEvent" => {
            let data = serde_json::from_str::<serde_json::Value>(&raw_data_str(&ev.data)).unwrap_or_default();
            let username = data["subscription"]["username"].as_str().or_else(|| data["username"].as_str()).unwrap_or("alguien");
            let months   = data["subscription"]["month"].as_u64().unwrap_or(1);
            let gifted   = data["subscription"]["gifted"].as_bool().unwrap_or(false);
            info!("[Kick][{}] Sub: {username} ({months}m, gifted={gifted})", ch.slug);
            alert_sub(username, months, gifted, ch, global).await;
        }
        "App\\Events\\LuckyUsersWhoGotGiftSubscriptionsEvent" => {
            let data    = serde_json::from_str::<serde_json::Value>(&raw_data_str(&ev.data)).unwrap_or_default();
            let gifter  = data["gifted_by"].as_str().unwrap_or("alguien");
            let count   = data["usernames"].as_array().map(|a| a.len()).unwrap_or(1);
            info!("[Kick][{}] Gift subs: {gifter} regaló {count}", ch.slug);
            alert_gift_sub(gifter, count, ch, global).await;
        }
        "App\\Events\\StreamerIsLive" => {
            info!("[Kick][{}] EN VIVO", ch.slug);
            ns_emit(global, &ch.slug, "streamStatus", json!({"live": true}));
        }
        "App\\Events\\StreamerIsOffline" => {
            info!("[Kick][{}] OFFLINE", ch.slug);
            ns_emit(global, &ch.slug, "streamStatus", json!({"live": false}));
        }
        other if other.starts_with("pusher") || other.contains("subscription_succeeded") => {}
        other => { debug!("[Pusher][{}] Evento: {other}", ch.slug); }
    }
}

// ─── Alertas ──────────────────────────────────────────────────────────────────

async fn alert_follow(username: &str, ch: &Arc<ChannelState>, global: &Arc<AppState>) {
    let msg = format!("¡Gracias por el follow, {username}!");
    ch.tts_tx.send(tts::TtsQueueItem { text: msg.clone(), voice: "dalia".into() }).ok();
    ns_emit(global, &ch.slug, "kickAlert", json!({"type":"follow","username":username,"message":msg}));
}

async fn alert_sub(username: &str, months: u64, gifted: bool, ch: &Arc<ChannelState>, global: &Arc<AppState>) {
    let msg = if gifted { format!("¡{username} recibió una sub de regalo!")
    } else if months > 1 { format!("¡{username} se resuscribió por {months} meses!")
    } else { format!("¡{username} se suscribió al canal!") };
    ch.tts_tx.send(tts::TtsQueueItem { text: msg.clone(), voice: "dalia".into() }).ok();
    ns_emit(global, &ch.slug, "kickAlert", json!({"type":"sub","username":username,"months":months,"gifted":gifted,"message":&msg}));
    sender::send(&format!("🎉 ¡Gracias por la sub, @{username}!"), ch, global).await;
}

async fn alert_gift_sub(gifter: &str, count: usize, ch: &Arc<ChannelState>, global: &Arc<AppState>) {
    let msg = format!("¡{gifter} regaló {count} suscripciones!");
    ch.tts_tx.send(tts::TtsQueueItem { text: msg.clone(), voice: "dalia".into() }).ok();
    ns_emit(global, &ch.slug, "kickAlert", json!({"type":"giftsub","gifter":gifter,"count":count,"message":&msg}));
    sender::send(&format!("🎁 ¡{gifter} regaló {count} subs! ¡Gracias!"), ch, global).await;
}

// ─── Pusher auth ──────────────────────────────────────────────────────────────

async fn pusher_auth(http: &reqwest::Client, socket_id: &str, channel: &str, token: &str) -> Option<String> {
    if token.is_empty() { return None; }

    // Intentar con reqwest (puede ser bloqueado por Cloudflare en algunos casos)
    let body = format!("socket_id={}&channel_name={}", socket_id, channel);
    if let Ok(resp) = http
        .post("https://kick.com/broadcasting/auth")
        .header("Authorization", format!("Bearer {token}"))
        .header("Content-Type", "application/x-www-form-urlencoded")
        .header("Origin", "https://kick.com")
        .header("Referer", "https://kick.com/")
        .body(body)
        .send()
        .await
    {
        if resp.status().is_success() {
            if let Ok(json) = resp.json::<serde_json::Value>().await {
                if let Some(auth) = json["auth"].as_str() {
                    info!("[Pusher auth] OK via reqwest");
                    return Some(auth.to_string());
                }
            }
        }
    }

    // En Windows: fallback via PowerShell (Schannel TLS fingerprint)
    #[cfg(windows)]
    {
        if let Some(auth) = pusher_auth_via_powershell(socket_id, channel, token).await {
            return Some(auth);
        }
    }

    None
}

#[cfg(windows)]
async fn pusher_auth_via_powershell(socket_id: &str, channel: &str, token: &str) -> Option<String> {
    let ps = r#"
$token=$env:KICK_TOKEN;$sid=$env:KICK_SOCKET_ID;$ch=$env:KICK_CHANNEL
$headers=@{'Authorization'="Bearer $token";'Origin'='https://kick.com';'Referer'='https://kick.com/'}
$body="socket_id=$sid&channel_name=$ch"
try{$r=Invoke-RestMethod -Uri 'https://kick.com/broadcasting/auth' -Method POST -Headers $headers -ContentType 'application/x-www-form-urlencoded' -Body $body -UserAgent 'Mozilla/5.0 (Windows NT 10.0; Win64; x64) Chrome/124.0.0.0';if($r.auth){$r.auth}else{""}}catch{""}
"#;
    let (sid, ch, tok) = (socket_id.to_string(), channel.to_string(), token.to_string());
    let result = tokio::task::spawn_blocking(move || {
        use std::os::windows::process::CommandExt;
        std::process::Command::new("powershell")
            .args(["-NoProfile", "-NonInteractive", "-Command", ps])
            .env("KICK_TOKEN", &tok).env("KICK_SOCKET_ID", &sid).env("KICK_CHANNEL", &ch)
            .creation_flags(0x08000000)
            .output()
    }).await.ok()?.ok()?;
    let out = String::from_utf8_lossy(&result.stdout).trim().to_string();
    if out.is_empty() || !out.contains(':') { None } else { Some(out) }
}

fn raw_data_str(data: &Option<serde_json::Value>) -> String {
    match data {
        Some(serde_json::Value::String(s)) => s.clone(),
        Some(v) => v.to_string(),
        None    => "{}".to_string(),
    }
}
