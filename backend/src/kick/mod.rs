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

const PUSHER_KEY: &str = "32cbd69e4b950bf97679";

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

    loop {
        match connect_once(&http, &state).await {
            Ok(_)  => warn!("Kick: conexión cerrada, reconectando en 5s…"),
            Err(e) => error!("Kick: {e} — reconectando en 5s…"),
        }
        sleep(Duration::from_secs(5)).await;
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

    // Conectar a Pusher — into_client_request() genera Sec-WebSocket-Key automáticamente,
    // luego añadimos Origin para que Pusher no rechace la conexión.
    let pusher_host = std::env::var("PUSHER_HOST")
        .unwrap_or_else(|_| "ws-us2.pusher.com".into());
    let ws_url = format!(
        "wss://{pusher_host}/app/{PUSHER_KEY}\
         ?protocol=7&client=js&version=8.5.0&flash=false"
    );
    info!("Conectando a Pusher: {pusher_host}");
    let mut request = {
        use tokio_tungstenite::tungstenite::client::IntoClientRequest;
        ws_url.as_str().into_client_request().map_err(|e| format!("WS request: {e}"))?
    };
    request.headers_mut().insert(
        tokio_tungstenite::tungstenite::http::header::ORIGIN,
        tokio_tungstenite::tungstenite::http::HeaderValue::from_static("https://kick.com"),
    );
    let (ws, _) = connect_async(request).await.map_err(|e| format!("WS: {e}"))?;
    let (mut tx, mut rx) = ws.split();

    // ── Esperar connection_established ────────────────────────────────────────
    let socket_id = wait_for_socket_id(&mut rx).await
        .ok_or("No se recibió connection_established de Pusher")?;

    info!("Pusher socket_id={socket_id}");

    // ── Chat (canal público — no requiere Pusher auth) ────────────────────────
    // "chatrooms.{id}.v2" es público. "private-chatrooms.{id}.v2" requiere
    // auth de sesión web que no acepta tokens OAuth.
    let chat_channel = format!("chatrooms.{}.v2", info.chatroom_id);
    subscribe(&mut tx, &chat_channel, None).await?;
    info!("Suscrito a {chat_channel}");

    // ── Eventos del canal (follows, subs, stream live/offline) ───────────────
    let events_channel = format!("channel.{}", info.slug);
    subscribe(&mut tx, &events_channel, None).await?;
    info!("Suscrito a {events_channel}");

    // ── Loop de mensajes ──────────────────────────────────────────────────────
    while let Some(msg) = rx.next().await {
        match msg {
            Ok(Message::Text(raw))    => on_event(&raw, &mut tx, state, &info.slug).await,
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
    let Ok(ev) = serde_json::from_str::<PusherEvent>(raw) else { return };

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
            let Ok(msg)  = serde_json::from_str::<KickChatMsg>(&data_str) else { return };

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

// ─── Utilidad ─────────────────────────────────────────────────────────────────

fn raw_data_str(data: &Option<serde_json::Value>) -> String {
    match data {
        Some(serde_json::Value::String(s)) => s.clone(),
        Some(v) => v.to_string(),
        None    => "{}".to_string(),
    }
}
