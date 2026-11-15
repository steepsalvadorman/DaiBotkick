use crate::{
    commands,
    kick::ingest::role as kick_role,
    platform::{is_blocked, Platform},
    server::ns_emit,
    state::AppState,
    tts,
};
use axum::{
    body::Bytes,
    extract::State,
    http::{HeaderMap, StatusCode},
};
use base64::{engine::general_purpose::STANDARD, Engine};
use rsa::{
    pkcs1v15::{Signature, VerifyingKey},
    pkcs8::DecodePublicKey,
    signature::Verifier,
    RsaPublicKey,
};
use sha2::Sha256;
use std::sync::Arc;

// Public verification key published by Kick; this is not a secret.
// Fallback only: at startup the current key is fetched from Kick (it can rotate).
const KICK_PUBLIC_KEY: &str =
    "-----BEGIN PUBLIC KEY-----\nMIIBIjANBgkqhkiG9w0BAQEFAAOCAQ8AMIIBCgKCAQEA0C0tthITvk/EjIxCGCko\nYrxM7eqP4GDnUyP4BnfgJ9yaHqniNfraxTKeRv7TGkOOZviow2zcx/YP9waURfHd\ncZOHU+EKA3lSFdMpezLiDGaym+FxR0iXAFZXE9VBdCCOyBeK81/m3mGScGVBNumt\n6pGCZYU9DCn5oqnC6RC5pUnlHnJp+TOXW6z8Silr4Y81a/66b0FAJ6EGUVXmXXgP\nFXQRTmJcLM4EgCXfNXLwExzr2MtowBwp5PYD6Usl7uZcnMIPutPdXJ0JnvqrztFC\nQTvrGMxzKLKLcKQTG159jfHGJ4wKSeenvwXN8jaVJAtW7wRAooRRT8Kho7Axe8jp\nqQIDAQAB\n-----END PUBLIC KEY-----";

pub fn public_key() -> RsaPublicKey {
    RsaPublicKey::from_public_key_pem(KICK_PUBLIC_KEY).expect("valid Kick public key")
}

/// Descarga la clave vigente de Kick; si falla, usa la incluida en el binario.
pub async fn fetch_public_key(http: &reqwest::Client, api: &str) -> RsaPublicKey {
    let fetched = async {
        let json: serde_json::Value = http
            .get(format!("{api}/public-key"))
            .send()
            .await
            .ok()?
            .error_for_status()
            .ok()?
            .json()
            .await
            .ok()?;
        RsaPublicKey::from_public_key_pem(json["data"]["public_key"].as_str()?).ok()
    }
    .await;
    match fetched {
        Some(key) => {
            tracing::info!("[Webhook] Clave pública de Kick actualizada");
            key
        }
        None => {
            tracing::warn!("[Webhook] No se pudo descargar la clave de Kick; usando la incluida");
            public_key()
        }
    }
}

fn header<'a>(headers: &'a HeaderMap, name: &str) -> Result<&'a str, StatusCode> {
    headers
        .get(name)
        .and_then(|v| v.to_str().ok())
        .filter(|v| !v.is_empty())
        .ok_or(StatusCode::UNAUTHORIZED)
}

pub fn verify(
    key: &RsaPublicKey,
    headers: &HeaderMap,
    body: &[u8],
    now: i64,
) -> Result<String, StatusCode> {
    let id = header(headers, "Kick-Event-Message-Id")?;
    if id.len() > 128 {
        return Err(StatusCode::UNAUTHORIZED);
    }
    let timestamp = header(headers, "Kick-Event-Message-Timestamp")?;
    let sent = chrono::DateTime::parse_from_rfc3339(timestamp)
        .map_err(|_| StatusCode::UNAUTHORIZED)?
        .timestamp();
    if now.abs_diff(sent) > 300 {
        return Err(StatusCode::UNAUTHORIZED);
    }
    let signature = STANDARD
        .decode(header(headers, "Kick-Event-Signature")?)
        .map_err(|_| StatusCode::UNAUTHORIZED)?;
    let signature =
        Signature::try_from(signature.as_slice()).map_err(|_| StatusCode::UNAUTHORIZED)?;
    let mut message = format!("{id}.{timestamp}.").into_bytes();
    message.extend_from_slice(body);
    VerifyingKey::<Sha256>::new(key.clone())
        .verify(&message, &signature)
        .map_err(|_| StatusCode::UNAUTHORIZED)?;
    Ok(id.to_owned())
}

pub async fn receive(
    State(app): State<Arc<AppState>>,
    headers: HeaderMap,
    body: Bytes,
) -> StatusCode {
    let now = chrono::Utc::now().timestamp();
    let id = match verify(&app.webhook_key, &headers, &body, now) {
        Ok(id) => id,
        Err(status) => {
            tracing::warn!("[Webhook] Firma rechazada ({status})");
            app.metrics
                .webhook_rejected
                .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            return status;
        }
    };
    let event_type = match header(&headers, "Kick-Event-Type") {
        Ok(value) => value,
        Err(status) => {
            return status;
        }
    };
    if serde_json::from_slice::<serde_json::Value>(&body).is_err() {
        return StatusCode::BAD_REQUEST;
    }
    let Ok(payload) = String::from_utf8(body.to_vec()) else {
        return StatusCode::BAD_REQUEST;
    };
    match
        sqlx
            ::query(
                "INSERT INTO webhook_events (platform,message_id,received_at,payload,event_type) VALUES ('kick',$1,$2,$3,$4) ON CONFLICT DO NOTHING"
            )
            .bind(id)
            .bind(now)
            .bind(payload)
            .bind(event_type)
            .execute(&app.db).await
    {
        Ok(result) => {
            tracing::info!("[Webhook] Evento recibido: {event_type}");
            app.metrics.webhook_received.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            if result.rows_affected() == 0 { app.metrics.webhook_duplicates.fetch_add(1, std::sync::atomic::Ordering::Relaxed); }
            StatusCode::OK
        }
        Err(e) => {
            tracing::error!("[Webhook] No se pudo guardar evento: {e}");
            StatusCode::SERVICE_UNAVAILABLE
        }
    }
}

/// Durable inbox: acknowledge after insertion, process sequentially and retry after a crash.
pub async fn worker(app: Arc<AppState>) {
    let mut next_cleanup = tokio::time::Instant::now();
    loop {
        let work = async {
            for _ in 0..50 {
                let mut transaction = app.db.begin().await?;
                let row = sqlx::query_as::<_, (String, String, String, String)>(
                "SELECT platform,message_id,payload,event_type FROM webhook_events WHERE NOT processed ORDER BY received_at,message_id LIMIT 1 FOR UPDATE SKIP LOCKED"
            ).fetch_optional(&mut *transaction).await?;
                let Some((platform, id, payload, kind)) = row else {
                    transaction.commit().await?;
                    break;
                };
                let json: serde_json::Value =
                    serde_json::from_str(&payload).expect("validated inbox JSON");
                match Platform::from_db(&platform) {
                    Some(Platform::Kick) => process(&app, &kind, &json).await,
                    // Los eventos de Twitch se procesan a partir de la etapa 4 del plan.
                    _ => tracing::warn!("[Webhook] Evento de {platform} sin procesador: {kind}"),
                }
                sqlx::query(
                    "UPDATE webhook_events SET processed=TRUE WHERE platform=$1 AND message_id=$2",
                )
                .bind(&platform)
                .bind(id)
                .execute(&mut *transaction)
                .await?;
                transaction.commit().await?;
                app.metrics
                    .webhook_processed
                    .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            }
            if tokio::time::Instant::now() >= next_cleanup {
                sqlx::query("DELETE FROM webhook_events WHERE processed AND received_at < $1")
                    .bind(chrono::Utc::now().timestamp() - 86400)
                    .execute(&app.db)
                    .await?;
                next_cleanup = tokio::time::Instant::now() + std::time::Duration::from_secs(300);
            }
            Ok::<(), sqlx::Error>(())
        };
        tokio::select! {
            _ = app.shutdown.cancelled() => return,
            result = work => if let Err(e) = result { tracing::error!("[Webhook] Inbox: {e}"); }
        }
        tokio::select! {
            _ = app.shutdown.cancelled() => return,
            _ = tokio::time::sleep(std::time::Duration::from_millis(250)) => {}
        }
    }
}

async fn process(app: &Arc<AppState>, kind: &str, json: &serde_json::Value) {
    let Some(id) = json["broadcaster"]["user_id"]
        .as_u64()
        .or_else(|| json["broadcaster_user_id"].as_u64())
    else {
        return;
    };
    let channel = app
        .user_id_to_key
        .get(&(Platform::Kick, id))
        .and_then(|key| app.channels.get(key.value()).map(|c| c.clone()));
    let Some(ch) = channel else {
        return;
    };
    match kind {
        "chat.message.sent" => {
            let Some(msg) = crate::kick::ingest::chat_message(json, id) else {
                return;
            };
            // El bot escribe como DaiBot: su eco no se muestra ni se procesa, venga de la app o del canal.
            if crate::kick::sender::is_own_echo(&ch, &msg.text).await {
                return;
            }
            // Un baneado (incluso si es moderador) solo aparece como texto en el chat del overlay.
            let banned = is_blocked(&ch, &msg.username, msg.role).await;
            // El dock de chat de OBS ve todo, comandos incluidos, con color e insignias.
            ns_emit(
                app,
                &ch,
                "dockChat",
                serde_json::json!({
                    "user": msg.username,
                    "content": msg.text,
                    "textOnly": banned,
                    "color": msg.color,
                    "badges": msg.badges,
                }),
            );
            // Los comandos (!play, !s…) no se muestran en el chat del overlay.
            if !msg.text.starts_with('!') {
                ns_emit(
                    app,
                    &ch,
                    "chatMessage",
                    serde_json::json!({"user":msg.username,"content":msg.text,"textOnly":banned}),
                );
            }
            if !banned {
                commands::handle(&msg, &ch, app).await;
            }
        }
        "channel.followed" => {
            let username = json["follower"]["username"].as_str().unwrap_or("alguien");
            // Evita que un baneado dispare voces y alertas siguiendo y dejando de seguir.
            if is_blocked(&ch, username, kick_role(&json["follower"], id)).await {
                return;
            }
            let message = format!("¡Gracias por el follow, {username}!");
            tts::enqueue(&ch, &message, "dalia");
            ns_emit(
                app,
                &ch,
                "kickAlert",
                serde_json::json!({"type":"follow","username":username,"message":message}),
            );
        }
        "channel.subscription.new" | "channel.subscription.renewal" => {
            let username = json["subscriber"]["username"].as_str().unwrap_or("alguien");
            let months = json["duration"]
                .as_u64()
                .or_else(|| json["months"].as_u64())
                .unwrap_or(1);
            let message = format!("¡{username} se suscribió por {months} meses!");
            tts::enqueue(&ch, &message, "dalia");
            ns_emit(
                app,
                &ch,
                "kickAlert",
                serde_json::json!({"type":"sub","username":username,"months":months,"message":message}),
            );
        }
        "channel.subscription.gifts" => {
            let username = json["gifter"]["username"].as_str().unwrap_or("alguien");
            let count = json["giftees"].as_array().map_or(0, Vec::len);
            let message = format!("¡{username} regaló {count} suscripciones!");
            tts::enqueue(&ch, &message, "dalia");
            ns_emit(
                app,
                &ch,
                "kickAlert",
                serde_json::json!({"type":"giftsub","message":message}),
            );
        }
        "livestream.status.updated" => {
            let live = json["is_live"].as_bool().unwrap_or(false);
            *ch.live_since.write().await = if live {
                json["started_at"]
                    .as_str()
                    .and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok())
                    .map(|t| t.with_timezone(&chrono::Utc))
            } else {
                None
            };
            ns_emit(
                app,
                &ch,
                "streamStatus",
                serde_json::json!({"live":live,"startedAt":ch.live_since.read().await.map(|t| t.to_rfc3339())}),
            );
        }
        _ => tracing::debug!("[Webhook] Ignorando tipo {kind}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rsa::{
        pkcs1v15::SigningKey,
        signature::{SignatureEncoding, Signer},
        RsaPrivateKey,
    };

    #[test]
    fn signed_body_is_verified_and_tampering_replays_rejected() {
        let private = RsaPrivateKey::new(&mut rand::thread_rng(), 2048).unwrap();
        let key = RsaPublicKey::from(&private);
        let timestamp = "2026-10-03T12:00:00Z";
        let now = chrono::DateTime::parse_from_rfc3339(timestamp)
            .unwrap()
            .timestamp();
        let body = br#"{"content":"hola"}"#;
        let mut message = format!("event.{timestamp}.").into_bytes();
        message.extend_from_slice(body);
        let signature = SigningKey::<Sha256>::new(private).sign(&message);
        let mut headers = HeaderMap::new();
        headers.insert("Kick-Event-Message-Id", "event".parse().unwrap());
        headers.insert("Kick-Event-Message-Timestamp", timestamp.parse().unwrap());
        headers.insert(
            "Kick-Event-Signature",
            STANDARD.encode(signature.to_bytes()).parse().unwrap(),
        );
        assert_eq!(verify(&key, &headers, body, now).unwrap(), "event");
        assert!(verify(&key, &headers, b"altered", now).is_err());
        assert!(verify(&key, &headers, body, now + 301).is_err());
        assert!(verify(&key, &headers, body, now - 301).is_err());
        headers.remove("Kick-Event-Signature");
        assert!(verify(&key, &headers, body, now).is_err());
    }

    #[tokio::test]
    async fn banned_users_only_chat_as_text_while_streamer_keeps_access() {
        let pool = sqlx::postgres::PgPoolOptions::new()
            .connect_lazy("postgres://unused:unused@127.0.0.1/unused")
            .unwrap();
        let app = crate::test_support::app(pool);
        let (ch, _rx) = crate::test_support::channel("alpha", 7);
        app.channels.insert("alpha".into(), ch.clone());
        app.user_id_to_key
            .insert((Platform::Kick, 7), "alpha".into());
        ch.banned.write().await.extend([
            "troll".to_string(),
            "badmod".to_string(),
            "alpha".to_string(),
        ]);
        ch.sorteo.lock().await.open = true;
        let message = |name: &str, user_id: u64, badge: &str| {
            serde_json::json!({
                "broadcaster": {"user_id": 7},
                "sender": {"user_id": user_id, "username": name, "identity": {"badges": [{"type": badge}]}},
                "content": "!participar",
            })
        };
        process(&app, "chat.message.sent", &message("Troll", 50, "")).await;
        process(
            &app,
            "chat.message.sent",
            &message("BadMod", 51, "moderator"),
        )
        .await;
        // Si el streamer quedara en la lista por error, su cuenta sigue funcionando.
        process(&app, "chat.message.sent", &message("Alpha", 7, "")).await;
        process(&app, "chat.message.sent", &message("viewer", 52, "")).await;
        assert_eq!(ch.sorteo.lock().await.participants, vec!["Alpha", "viewer"]);
        let user =
            |name: &str, user_id: u64| serde_json::json!({"username": name, "user_id": user_id});
        assert!(!is_blocked(&ch, "SeniorDai", kick_role(&user("SeniorDai", 1), 7)).await);
        assert!(is_blocked(&ch, "@TROLL", kick_role(&user("@TROLL", 1), 7)).await);
        assert!(!is_blocked(&ch, "", kick_role(&serde_json::json!({"user_id": 1}), 7)).await);
    }

    #[test]
    fn kick_key_parses() {
        assert_eq!(rsa::traits::PublicKeyParts::size(&public_key()), 256);
    }
}
