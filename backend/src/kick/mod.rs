pub mod api;
pub mod sender;

use crate::{commands, tts, AppState};
use futures_util::{SinkExt, StreamExt};
use serde::Deserialize;
use serde_json::json;
use std::sync::{atomic::Ordering, Arc};
use tokio::time::{sleep, Duration};
use tokio_tungstenite::{connect_async, tungstenite::Message};
use tracing::{debug, error, info, warn};

// ─── Auto-refresh de tokens OAuth ─────────────────────────────────────────────

fn unix_now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

/// Intenta renovar el access_token con el refresh_token.
/// Devuelve true si tuvo éxito.
pub async fn refresh_access_token(state: &Arc<AppState>) -> bool {
    let client_id     = &state.config.client_id;
    let client_secret = &state.config.client_secret;
    let refresh_tok   = state.refresh_token_val.read().await.clone();

    if client_id.is_empty() || client_secret.is_empty() || refresh_tok.is_empty() {
        warn!("[OAuth] No se puede renovar: faltan client_id/secret/refresh_token en .env");
        return false;
    }

    let params = [
        ("grant_type",    "refresh_token"),
        ("refresh_token", refresh_tok.as_str()),
        ("client_id",     client_id.as_str()),
        ("client_secret", client_secret.as_str()),
    ];

    let resp = state.http
        .post("https://id.kick.com/oauth/token")
        .form(&params)
        .send()
        .await;

    let r = match resp {
        Ok(r) => r,
        Err(e) => { error!("[OAuth] Error de red al renovar token: {e}"); return false; }
    };

    if !r.status().is_success() {
        error!("[OAuth] Refresh falló: {}", r.status());
        return false;
    }

    let data: serde_json::Value = match r.json().await {
        Ok(d) => d,
        Err(e) => { error!("[OAuth] Respuesta de refresh no válida: {e}"); return false; }
    };

    let Some(new_access) = data["access_token"].as_str() else {
        error!("[OAuth] Respuesta sin access_token: {data}");
        return false;
    };
    let new_refresh   = data["refresh_token"].as_str().unwrap_or(&refresh_tok);
    let expires_in    = data["expires_in"].as_u64().unwrap_or(7200);

    *state.access_token.write().await      = new_access.to_string();
    *state.refresh_token_val.write().await = new_refresh.to_string();

    info!("[OAuth] Token renovado correctamente, expira en {expires_in}s");
    true
}

/// Tarea de fondo: renueva el token 10 minutos antes de que expire.
pub async fn token_refresh_loop(state: Arc<AppState>, initial_expires: u64) {
    let mut expires = initial_expires;
    loop {
        let now = unix_now();
        // Dormir hasta 10 minutos antes de la expiración (mínimo 60s)
        let sleep_secs = if expires > now + 610 {
            expires - now - 600
        } else {
            60
        };
        sleep(Duration::from_secs(sleep_secs)).await;

        info!("[OAuth] Renovando token proactivamente...");
        if refresh_access_token(&state).await {
            // Actualizar el tiempo de expiración para el próximo ciclo
            expires = unix_now() + 7200;
        }
        // Si falla, reintentar en 60s
    }
}

// Key primaria en us2 (conecta pero no entrega mensajes sin auth)
// También intentamos otros clusters con la key alternativa
const PUSHER_KEY_PRIMARY: &str = "32cbd69e4b950bf97679";
const PUSHER_KEY_ALT: &str     = "eb1d5f283081a78b932c";

// Lista de (host, key) a probar en orden hasta que uno funcione
const PUSHER_ENDPOINTS: &[(&str, &str)] = &[
    ("ws-us2.pusher.com",  PUSHER_KEY_PRIMARY),
    ("ws-eu.pusher.com",   PUSHER_KEY_ALT),
    ("ws-mt1.pusher.com",  PUSHER_KEY_ALT),
    ("ws-ap1.pusher.com",  PUSHER_KEY_ALT),
    ("ws-ap3.pusher.com",  PUSHER_KEY_ALT),
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
struct KickSender {
    username: String,
}

pub async fn run(state: Arc<AppState>) {
    let http = state.http.clone();

    // Lanzar polling de chat como respaldo al Pusher
    let poll_state = state.clone();
    tokio::spawn(async move {
        poll_chat_loop(poll_state).await;
    });

    // Suscribir a eventos de Kick vía EventSub
    let sub_state = state.clone();
    tokio::spawn(async move {
        let broadcaster_id = loop {
            let id = *sub_state.channel_id.read().await;
            if let Some(id) = id { break id; }
            sleep(Duration::from_secs(2)).await;
        };
        // Pequeño delay para que ngrok esté activo
        sleep(Duration::from_secs(3)).await;
        subscribe_kick_events(&sub_state, broadcaster_id).await;
    });

    loop {
        match connect_once(&http, &state).await {
            Ok(_)  => warn!("Kick: conexión cerrada, reconectando en 5s…"),
            Err(e) => error!("Kick: {e} — reconectando en 5s…"),
        }
        sleep(Duration::from_secs(5)).await;
    }
}


/// Suscribe a eventos de Kick via EventSub API.
async fn subscribe_kick_events(state: &Arc<AppState>, broadcaster_user_id: u64) {
    let token = state.access_token.read().await.clone();
    let url = "https://api.kick.com/public/v1/events/subscriptions";

    let body = serde_json::json!({
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

    match state.http
        .post(url)
        .header("Authorization", format!("Bearer {token}"))
        .json(&body)
        .send()
        .await
    {
        Ok(r) => {
            let status = r.status();
            let text   = r.text().await.unwrap_or_default();
            if status.is_success() {
                info!("[EventSub] Suscripciones creadas: {text:.300}");
            } else {
                warn!("[EventSub] Error {status}: {text:.300}");
            }
        }
        Err(e) => warn!("[EventSub] Error de red: {e}"),
    }
}

/// Polling de mensajes de chat vía API pública como respaldo al Pusher.
/// Prueba varios endpoints hasta encontrar uno que funcione.
async fn poll_chat_loop(state: Arc<AppState>) {
    // Esperar a que el chatroom_id esté disponible
    let chatroom_id = loop {
        {
            let id = state.chatroom_id.read().await;
            if let Some(id) = *id { break id; }
        }
        sleep(Duration::from_secs(2)).await;
    };

    let candidates = [
        format!("https://api.kick.com/public/v1/chatrooms/{chatroom_id}/messages"),
        format!("https://api.kick.com/public/v1/channels/{chatroom_id}/messages"),
    ];

    info!("[Poll] Buscando endpoint de mensajes para chatroom {chatroom_id}...");

    // Descubrir qué URL funciona
    let mut active_url: Option<String> = None;
    for url in &candidates {
        let token = state.access_token.read().await.clone();
        match state.http.get(url)
            .header("Authorization", format!("Bearer {token}"))
            .send().await
        {
            Ok(r) => {
                let status = r.status();
                let body   = r.text().await.unwrap_or_default();
                if status.is_success() {
                    info!("[Poll] Endpoint OK: {url} → {body:.200}");
                    active_url = Some(url.clone());
                    break;
                } else {
                    info!("[Poll] {url} → {status}: {body:.200}");
                }
            }
            Err(e) => warn!("[Poll] {url} → red: {e}"),
        }
    }

    let Some(url) = active_url else {
        warn!("[Poll] Ningún endpoint de mensajes disponible en la API pública — solo Pusher activo");
        return;
    };

    info!("[Poll] Usando {url} (intervalo 1s)");
    let mut last_id: Option<String> = None;

    loop {
        sleep(Duration::from_secs(1)).await;

        let token = state.access_token.read().await.clone();
        let resp = match state.http.get(&url)
            .header("Authorization", format!("Bearer {token}"))
            .send().await
        {
            Ok(r)  => r,
            Err(e) => { warn!("[Poll] Error de red: {e}"); continue; }
        };

        if !resp.status().is_success() { continue; }

        let json: serde_json::Value = match resp.json().await {
            Ok(j)  => j,
            Err(e) => { warn!("[Poll] JSON inválido: {e}"); continue; }
        };

        // Procesar mensajes nuevos (array en data o en la raíz)
        let msgs = json.get("data")
            .and_then(|d| d.as_array())
            .or_else(|| json.as_array())
            .cloned()
            .unwrap_or_default();

        for msg in msgs.iter().rev() {
            let id = msg.get("id")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string())
                .or_else(|| msg["id"].as_u64().map(|n| n.to_string()));

            if id.as_deref() == last_id.as_deref() { break; }

            let username = msg["sender"]["username"].as_str()
                .or_else(|| msg["username"].as_str())
                .unwrap_or("?")
                .to_string();
            let content = msg["content"].as_str().unwrap_or("").trim().to_string();

            if content.is_empty() { continue; }

            info!("[CHAT-Poll] {username}: {content}");
            state.io.emit("chatMessage", serde_json::json!({ "user": &username, "content": &content })).ok();
            commands::handle(&username, &content, &state).await;
        }

        if let Some(first) = msgs.first() {
            last_id = first.get("id")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string())
                .or_else(|| first["id"].as_u64().map(|n| n.to_string()));
        }
    }
}

async fn connect_once(http: &reqwest::Client, state: &Arc<AppState>) -> Result<(), String> {
    let chan = &state.config.channel_name;

    // Obtener IDs del canal — si falla con 401 intentar renovar el token primero
    let token = state.access_token.read().await.clone();
    let info = match api::get_channel_info(http, chan, &token).await {
        Some(i) => i,
        None => {
            warn!("Kick: token inválido, intentando renovar...");
            if refresh_access_token(state).await {
                let new_token = state.access_token.read().await.clone();
                api::get_channel_info(http, chan, &new_token)
                    .await
                    .ok_or_else(|| format!("No se pudo obtener info del canal '{chan}' — corre --login para reautenticar"))?
            } else {
                return Err(format!("Token expirado y no se pudo renovar — corre daibot.exe --login"));
            }
        }
    };

    info!("Canal '{}' → channel_id={} chatroom_id={}", info.slug, info.channel_id, info.chatroom_id);

    // Guardar IDs en AppState para el sender
    *state.channel_id.write().await  = Some(info.channel_id);
    *state.chatroom_id.write().await = Some(info.chatroom_id);

    // Conectar probando cada endpoint (host+key) hasta encontrar uno que devuelva
    // connection_established. Si PUSHER_HOST está en .env, usarlo directamente.
    let override_host = std::env::var("PUSHER_HOST").ok();
    let endpoints_to_try: Vec<(&str, &str)> = if let Some(ref h) = override_host {
        vec![(h.as_str(), PUSHER_KEY_PRIMARY)]
    } else {
        PUSHER_ENDPOINTS.to_vec()
    };

    let mut found: Option<(
        futures_util::stream::SplitSink<tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>, Message>,
        futures_util::stream::SplitStream<tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>>,
        String, // socket_id
    )> = None;

    for (pusher_host, pusher_key) in &endpoints_to_try {
        let ws_url = format!(
            "wss://{pusher_host}/app/{pusher_key}\
             ?protocol=7&client=js&version=8.5.0&flash=false"
        );
        info!("Probando WebSocket: {pusher_host} (key={}...)", &pusher_key[..8]);

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
            Err(e) => { warn!("  {pusher_host}: error de red: {e}"); continue; }
        };
        let (tx, mut rx) = ws.split();

        match wait_for_socket_id(&mut rx).await {
            Some(sid) => {
                info!("  ✓ Conectado a {pusher_host} socket_id={sid}");
                found = Some((tx, rx, sid));
                break;
            }
            None => {
                info!("  ✗ {pusher_host}: no devolvió connection_established");
                continue;
            }
        }
    }

    let (mut tx, mut rx, socket_id) = found
        .ok_or_else(|| "Ningún endpoint de Pusher respondió correctamente".to_string())?;

    info!("Pusher socket_id={socket_id}");

    // ── Chat: intentar canal privado (autenticado) primero, luego el público ──
    // Kick dejó de entregar mensajes en el canal público sin auth. El canal
    // privado requiere un auth token de Kick obtenido con el access_token OAuth.
    let token = state.access_token.read().await.clone();
    let private_chat = format!("private-chatrooms.{}.v2", info.chatroom_id);
    let public_chat  = format!("chatrooms.{}.v2", info.chatroom_id);

    let chat_channel = match pusher_auth(http, &socket_id, &private_chat, &token).await {
        Some(auth) => {
            info!("Auth Pusher OK — usando canal privado {private_chat}");
            subscribe(&mut tx, &private_chat, Some(&auth)).await?;
            private_chat
        }
        None => {
            warn!("Auth Pusher falló — fallback a canal público {public_chat}");
            subscribe(&mut tx, &public_chat, None).await?;
            public_chat
        }
    };
    info!("Suscrito a {chat_channel}");

    // ── Eventos del canal (follows, subs, stream live/offline) ───────────────
    let events_channel = format!("channel.{}", info.slug);
    subscribe(&mut tx, &events_channel, None).await?;
    info!("Suscrito a {events_channel}");

    // ── Loop de mensajes ──────────────────────────────────────────────────────
    while let Some(msg) = rx.next().await {
        match msg {
            Ok(Message::Text(raw)) => {
                on_event(&raw, &mut tx, state, &info.slug).await;
            }
            Ok(Message::Ping(d))      => { tx.send(Message::Pong(d)).await.ok(); }
            Ok(Message::Close(_))     => { warn!("Pusher cerró la conexión"); break; }
            Err(e)                    => return Err(format!("WS: {e}")),
            _                         => {}
        }
    }
    Ok(())
}

// ─── Helpers de suscripción ───────────────────────────────────────────────────

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
                debug!("[Pusher] ← {raw}");
                let Ok(ev) = serde_json::from_str::<PusherEvent>(&raw) else {
                    warn!("[Pusher] Mensaje no-JSON durante handshake: {raw}");
                    continue
                };
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
                    "pusher:error" => {
                        error!("[Pusher] Error del servidor: {raw}");
                        return None;
                    }
                    other => debug!("[Pusher] Evento durante handshake: {other}"),
                }
            }
            Ok(Message::Close(frame)) => {
                warn!("[Pusher] Conexión cerrada durante handshake: {:?}", frame);
                return None;
            }
            Ok(other) => debug!("[Pusher] Mensaje no-texto durante handshake: {:?}", other),
            Err(e)    => { warn!("[Pusher] Error WS en handshake: {e}"); return None; }
        }
    }
    warn!("[Pusher] Stream terminó sin recibir connection_established");
    None
}


// ─── Manejador de eventos Pusher ──────────────────────────────────────────────

async fn on_event(
    raw:   &str,
    tx:    &mut (impl SinkExt<Message, Error = tokio_tungstenite::tungstenite::Error> + Unpin),
    state: &Arc<AppState>,
    slug:  &str,
) {
    let Ok(ev) = serde_json::from_str::<PusherEvent>(raw) else {
        debug!("[Pusher] JSON no parseable: {raw}");
        return;
    };

    // Log de todos los eventos que llegan (visible con RUST_LOG=debug)
    debug!("[Pusher] evento={} data={:?}", ev.event, ev.data);

    match ev.event.as_str() {

        // ── Protocolo Pusher ─────────────────────────────────────────────────
        "pusher:ping" => {
            tx.send(Message::Text(
                json!({"event":"pusher:pong","data":{}}).to_string()
            )).await.ok();
        }

        // ── Chat ─────────────────────────────────────────────────────────────
        "App\\Events\\ChatMessageEvent" => {
            let data_str = raw_data_str(&ev.data);
            let Ok(msg) = serde_json::from_str::<KickChatMsg>(&data_str) else {
                warn!("[Kick] ChatMessageEvent no parseable — data cruda: {data_str}");
                return;
            };

            let username = msg.sender.username.clone();
            let content  = msg.content.trim().to_string();

            state.io.emit("chatMessage", json!({ "user": &username, "content": &content })).ok();
            commands::handle(&username, &content, state).await;
        }

        // ── Seguidores actualizados en tiempo real (elimina polling 60s) ─────
        "App\\Events\\FollowersUpdated" => {
            let data = serde_json::from_str::<serde_json::Value>(&raw_data_str(&ev.data))
                .unwrap_or_default();

            if let Some(count) = data["followersCount"].as_u64()
                .or_else(|| data["followers_count"].as_u64())
            {
                state.followers.store(count, Ordering::Relaxed);
                state.io.emit("followGoal", json!({
                    "current": count,
                    "goal":    state.config.follow_goal,
                })).ok();
                info!("[Kick] Seguidores actualizados: {count}");
            }
        }

        // ── Nuevo follow ─────────────────────────────────────────────────────
        "App\\Events\\FollowEvent" => {
            let data = serde_json::from_str::<serde_json::Value>(&raw_data_str(&ev.data))
                .unwrap_or_default();
            let username = data["user_username"].as_str()
                .or_else(|| data["username"].as_str())
                .unwrap_or("alguien");

            info!("[Kick] Nuevo follow: {username}");
            alert_follow(username, slug, state).await;
        }

        // ── Nueva suscripción ────────────────────────────────────────────────
        "App\\Events\\SubscriptionEvent" => {
            let data = serde_json::from_str::<serde_json::Value>(&raw_data_str(&ev.data))
                .unwrap_or_default();

            // El username puede estar en varias rutas según el tipo de sub
            let username = data["subscription"]["username"].as_str()
                .or_else(|| data["username"].as_str())
                .unwrap_or("alguien");
            let months = data["subscription"]["month"].as_u64().unwrap_or(1);
            let gifted = data["subscription"]["gifted"].as_bool().unwrap_or(false);

            info!("[Kick] Suscripción: {username} ({months} mes(es), gifted={gifted})");
            alert_sub(username, months, gifted, slug, state).await;
        }

        // ── Subs regaladas ────────────────────────────────────────────────────
        "App\\Events\\LuckyUsersWhoGotGiftSubscriptionsEvent" => {
            let data = serde_json::from_str::<serde_json::Value>(&raw_data_str(&ev.data))
                .unwrap_or_default();
            let gifter = data["gifted_by"].as_str().unwrap_or("alguien");
            let count  = data["usernames"].as_array().map(|a| a.len()).unwrap_or(1);

            info!("[Kick] Gift subs: {gifter} regaló {count} subs");
            alert_gift_sub(gifter, count, state).await;
        }

        // ── Stream live/offline ───────────────────────────────────────────────
        "App\\Events\\StreamerIsLive" => {
            info!("[Kick] Stream EN VIVO");
            state.io.emit("streamStatus", json!({ "live": true })).ok();
        }
        "App\\Events\\StreamerIsOffline" => {
            info!("[Kick] Stream OFFLINE");
            state.io.emit("streamStatus", json!({ "live": false })).ok();
        }

        // Eventos de protocolo (subscription_succeeded, etc.) — ignorar
        other if other.starts_with("pusher") || other.contains("subscription_succeeded") => {}

        other => {
            tracing::debug!("[Pusher] Evento no manejado: {other}");
        }
    }
}

// ─── Alertas ──────────────────────────────────────────────────────────────────

async fn alert_follow(username: &str, _slug: &str, state: &Arc<AppState>) {
    let msg = format!("¡Gracias por el follow, {username}!");

    // TTS
    state.tts_tx.send(tts::TtsQueueItem {
        text:  msg.clone(),
        voice: "dalia".into(),
    }).ok();

    // Overlay alert
    state.io.emit("kickAlert", json!({
        "type":     "follow",
        "username": username,
        "message":  msg,
    })).ok();
}

async fn alert_sub(username: &str, months: u64, gifted: bool, _slug: &str, state: &Arc<AppState>) {
    let msg = if gifted {
        format!("¡{username} recibió una suscripción de regalo!")
    } else if months > 1 {
        format!("¡{username} se resuscribió por {months} meses!")
    } else {
        format!("¡{username} se suscribió al canal!")
    };

    state.tts_tx.send(tts::TtsQueueItem {
        text:  msg.clone(),
        voice: "dalia".into(),
    }).ok();

    state.io.emit("kickAlert", json!({
        "type":     "sub",
        "username": username,
        "months":   months,
        "gifted":   gifted,
        "message":  msg,
    })).ok();

    // Mensaje en el chat
    sender::send(&format!("🎉 ¡Gracias por la sub, @{username}!"), state).await;
}

async fn alert_gift_sub(gifter: &str, count: usize, state: &Arc<AppState>) {
    let msg = format!("¡{gifter} regaló {count} suscripciones!");

    state.tts_tx.send(tts::TtsQueueItem {
        text:  msg.clone(),
        voice: "dalia".into(),
    }).ok();

    state.io.emit("kickAlert", json!({
        "type":    "giftsub",
        "gifter":  gifter,
        "count":   count,
        "message": msg,
    })).ok();

    sender::send(&format!("🎁 ¡{gifter} regaló {count} subs! ¡Gracias!"), state).await;
}

// ─── Autenticación Pusher para canales privados ───────────────────────────────

/// Obtiene el auth token de Kick para suscribirse a canales privados de Pusher.
/// reqwest/rustls tiene un fingerprint TLS que Cloudflare bloquea en kick.com.
/// PowerShell usa Windows Schannel (mismo TLS que Edge/IE) y pasa el filtro.
async fn pusher_auth(
    _http:     &reqwest::Client,
    socket_id: &str,
    channel:   &str,
    token:     &str,
) -> Option<String> {
    if token.is_empty() { return None; }

    // Primero intentar la API pública (no tiene fingerprint issue)
    // — mantenemos este intento por si Kick añade el endpoint en el futuro
    // (actualmente devuelve 404)

    // Usar PowerShell + Windows Schannel para bypass de Cloudflare TLS fingerprint
    #[cfg(windows)]
    {
        let auth = pusher_auth_via_powershell(socket_id, channel, token).await;
        if auth.is_some() {
            return auth;
        }
    }

    None
}

/// Llama a kick.com/broadcasting/auth usando PowerShell (Windows Schannel),
/// que tiene un fingerprint TLS diferente a reqwest/rustls, bypaseando Cloudflare.
#[cfg(windows)]
async fn pusher_auth_via_powershell(
    socket_id: &str,
    channel:   &str,
    token:     &str,
) -> Option<String> {
    let ps_script = r#"
$token     = $env:KICK_TOKEN
$socket_id = $env:KICK_SOCKET_ID
$channel   = $env:KICK_CHANNEL
$ua = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/124.0.0.0 Safari/537.36"
$headers = @{
    'Authorization' = "Bearer $token"
    'Origin'        = 'https://kick.com'
    'Referer'       = 'https://kick.com/'
    'Accept'        = 'application/json, text/plain, */*'
}
$body = "socket_id=$socket_id&channel_name=$channel"
try {
    $resp = Invoke-RestMethod -Uri 'https://kick.com/broadcasting/auth' `
        -Method POST `
        -Headers $headers `
        -ContentType 'application/x-www-form-urlencoded' `
        -Body $body `
        -UserAgent $ua
    if ($resp.auth) { $resp.auth } else { "" }
} catch {
    ""
}
"#;

    let socket_id = socket_id.to_string();
    let channel   = channel.to_string();
    let token     = token.to_string();

    let result = tokio::task::spawn_blocking(move || {
        use std::os::windows::process::CommandExt;
        std::process::Command::new("powershell")
            .args(["-NoProfile", "-NonInteractive", "-Command", ps_script])
            .env("KICK_TOKEN",     &token)
            .env("KICK_SOCKET_ID", &socket_id)
            .env("KICK_CHANNEL",   &channel)
            .creation_flags(0x08000000) // CREATE_NO_WINDOW
            .output()
    }).await.ok()?.ok()?;

    let stdout = String::from_utf8_lossy(&result.stdout).trim().to_string();
    let stderr = String::from_utf8_lossy(&result.stderr).trim().to_string();

    if !stderr.is_empty() {
        warn!("[Pusher auth PS] stderr: {stderr}");
    }

    if stdout.is_empty() || !stdout.contains(':') {
        warn!("[Pusher auth PS] respuesta vacía o inválida: {stdout:?}");
        return None;
    }

    info!("[Pusher auth PS] OK — auth={stdout:.40}...");
    Some(stdout)
}

fn url_encode(s: &str) -> String {
    s.bytes().flat_map(|b| match b {
        b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9'
        | b'-' | b'_' | b'.' | b'~' => vec![b as char],
        b' ' => vec!['+'],
        _ => format!("%{b:02X}").chars().collect(),
    }).collect()
}

// ─── Utilidad ─────────────────────────────────────────────────────────────────

fn raw_data_str(data: &Option<serde_json::Value>) -> String {
    match data {
        Some(serde_json::Value::String(s)) => s.clone(),
        Some(v) => v.to_string(),
        None    => "{}".to_string(),
    }
}
