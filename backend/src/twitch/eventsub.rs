//! EventSub de Twitch por webhook: verificación de firmas, recepción en el inbox común y
//! creación de las suscripciones de cada canal.
//! <https://dev.twitch.tv/docs/eventsub/handling-webhook-events/>

use crate::state::{AppState, ChannelState};
use axum::{
    body::Bytes,
    extract::State,
    http::{header, HeaderMap, StatusCode},
    response::{IntoResponse, Response},
};
use hmac::{Hmac, Mac};
use sha2::Sha256;
use std::sync::Arc;

/// Twitch reenvía eventos hasta 10 minutos; uno más viejo se trata como repetición.
const MAX_AGE_SECS: u64 = 600;

#[derive(Debug, PartialEq)]
pub enum Delivery {
    /// Twitch confirma que la URL es nuestra: hay que devolver el `challenge`.
    Verification {
        challenge: String,
    },
    Notification {
        id: String,
        kind: String,
    },
    Revocation {
        kind: String,
        status: String,
    },
}

/// Comprueba la firma `sha256=HMAC(secreto, id + timestamp + cuerpo)` y la antigüedad del mensaje.
pub fn verify(
    secret: &str,
    headers: &HeaderMap,
    body: &[u8],
    now: i64,
) -> Result<Delivery, StatusCode> {
    let get = |name: &str| {
        headers
            .get(name)
            .and_then(|v| v.to_str().ok())
            .filter(|v| !v.is_empty())
            .ok_or(StatusCode::FORBIDDEN)
    };
    let id = get("Twitch-Eventsub-Message-Id")?;
    let timestamp = get("Twitch-Eventsub-Message-Timestamp")?;
    let signature = get("Twitch-Eventsub-Message-Signature")?
        .strip_prefix("sha256=")
        .and_then(decode_hex)
        .ok_or(StatusCode::FORBIDDEN)?;
    if id.len() > 128 {
        return Err(StatusCode::FORBIDDEN);
    }
    let mut mac =
        Hmac::<Sha256>::new_from_slice(secret.as_bytes()).expect("HMAC acepta cualquier clave");
    mac.update(id.as_bytes());
    mac.update(timestamp.as_bytes());
    mac.update(body);
    // `verify_slice` compara en tiempo constante.
    mac.verify_slice(&signature)
        .map_err(|_| StatusCode::FORBIDDEN)?;
    let sent = chrono::DateTime::parse_from_rfc3339(timestamp)
        .map_err(|_| StatusCode::FORBIDDEN)?
        .timestamp();
    if now.abs_diff(sent) > MAX_AGE_SECS {
        return Err(StatusCode::FORBIDDEN);
    }
    let json: serde_json::Value =
        serde_json::from_slice(body).map_err(|_| StatusCode::BAD_REQUEST)?;
    let kind = json["subscription"]["type"]
        .as_str()
        .unwrap_or("")
        .to_owned();
    match get("Twitch-Eventsub-Message-Type")? {
        "webhook_callback_verification" => Ok(Delivery::Verification {
            challenge: json["challenge"]
                .as_str()
                .ok_or(StatusCode::BAD_REQUEST)?
                .to_owned(),
        }),
        "notification" => Ok(Delivery::Notification {
            id: id.to_owned(),
            kind,
        }),
        "revocation" => Ok(Delivery::Revocation {
            kind,
            status: json["subscription"]["status"]
                .as_str()
                .unwrap_or("")
                .to_owned(),
        }),
        _ => Err(StatusCode::BAD_REQUEST),
    }
}

fn decode_hex(hex: &str) -> Option<Vec<u8>> {
    if !hex.len().is_multiple_of(2) {
        return None;
    }
    (0..hex.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(hex.get(i..i + 2)?, 16).ok())
        .collect()
}

/// `POST /twitch_webhook`: responde rápido y deja el evento en el inbox para procesarlo.
pub async fn receive(
    State(app): State<Arc<AppState>>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    let Some(tw) = &app.config.twitch else {
        return StatusCode::NOT_FOUND.into_response();
    };
    let now = chrono::Utc::now().timestamp();
    let delivery = match verify(&tw.eventsub_secret, &headers, &body, now) {
        Ok(delivery) => delivery,
        Err(status) => {
            tracing::warn!("[Twitch][Webhook] Firma rechazada ({status})");
            app.metrics
                .webhook_rejected
                .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            return status.into_response();
        }
    };
    match delivery {
        Delivery::Verification { challenge } => {
            tracing::info!("[Twitch][Webhook] Suscripción verificada");
            ([(header::CONTENT_TYPE, "text/plain")], challenge).into_response()
        }
        Delivery::Revocation { kind, status } => {
            // Pasa si el streamer quita el permiso a la app: vuelve a conectar su canal.
            tracing::warn!("[Twitch][Webhook] Twitch revocó {kind}: {status}");
            StatusCode::NO_CONTENT.into_response()
        }
        Delivery::Notification { id, kind } => {
            let Ok(payload) = String::from_utf8(body.to_vec()) else {
                return StatusCode::BAD_REQUEST.into_response();
            };
            let stored = sqlx::query(
                "INSERT INTO webhook_events (platform,message_id,received_at,payload,event_type) VALUES ('twitch',$1,$2,$3,$4) ON CONFLICT DO NOTHING",
            )
            .bind(&id)
            .bind(now)
            .bind(payload)
            .bind(&kind)
            .execute(&app.db)
            .await;
            match stored {
                Ok(result) => {
                    let metrics = &app.metrics;
                    metrics
                        .webhook_received
                        .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                    if result.rows_affected() == 0 {
                        metrics
                            .webhook_duplicates
                            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                    }
                    tracing::info!("[Twitch][Webhook] Evento recibido: {kind}");
                    StatusCode::NO_CONTENT.into_response()
                }
                Err(e) => {
                    tracing::error!("[Twitch][Webhook] No se pudo guardar evento: {e}");
                    StatusCode::SERVICE_UNAVAILABLE.into_response()
                }
            }
        }
    }
}

/// Token de app (`client_credentials`), el que exige EventSub por webhook. Se guarda en memoria
/// y se pide otro un minuto antes de que venza.
async fn app_token(app: &AppState) -> Option<String> {
    let mut cached = app.twitch_app_token.lock().await;
    let now = chrono::Utc::now().timestamp();
    if let Some((token, expires)) = cached.as_ref() {
        if *expires - 60 > now {
            return Some(token.clone());
        }
    }
    let (url, client_id, client_secret) =
        crate::oauth::token_endpoint(crate::platform::Platform::Twitch, app)?;
    let json: serde_json::Value = app
        .http
        .post(url)
        .form(&[
            ("client_id", client_id),
            ("client_secret", client_secret),
            ("grant_type", "client_credentials"),
        ])
        .send()
        .await
        .ok()?
        .error_for_status()
        .ok()?
        .json()
        .await
        .ok()?;
    let token = json["access_token"].as_str()?.to_owned();
    *cached = Some((
        token.clone(),
        now + json["expires_in"].as_i64().unwrap_or(3600),
    ));
    Some(token)
}

/// Suscripciones que necesita un canal: `(tipo, versión, condición)`. El chat va como la
/// cuenta bot, que debe estar autorizada; sin ella se crean las demás y se reintenta.
pub fn subscriptions(
    broadcaster_id: &str,
    bot_id: Option<&str>,
) -> Vec<(&'static str, &'static str, serde_json::Value)> {
    let only = serde_json::json!({ "broadcaster_user_id": broadcaster_id });
    let mut list = vec![
        (
            "channel.follow",
            "2",
            serde_json::json!({ "broadcaster_user_id": broadcaster_id, "moderator_user_id": broadcaster_id }),
        ),
        ("channel.subscribe", "1", only.clone()),
        ("channel.subscription.message", "1", only.clone()),
        ("channel.subscription.gift", "1", only.clone()),
        ("stream.online", "1", only.clone()),
        ("stream.offline", "1", only),
    ];
    if let Some(bot) = bot_id {
        list.insert(
            0,
            (
                "channel.chat.message",
                "1",
                serde_json::json!({ "broadcaster_user_id": broadcaster_id, "user_id": bot }),
            ),
        );
    }
    list
}

/// Crea las suscripciones del canal. Devuelve `true` cuando están todas, incluida la del chat.
/// Twitch responde 409 si ya existe una suscripción igual: cuenta como lista.
pub async fn subscribe(ch: &ChannelState, app: &AppState) -> bool {
    let (Some(tw), Some(broadcaster)) = (&app.config.twitch, *ch.channel_id.read().await) else {
        return false;
    };
    let Some(token) = app_token(app).await else {
        tracing::warn!("[Twitch][EventSub][{}] Sin token de app", ch.slug);
        return false;
    };
    let bot_id = app
        .twitch_bot
        .read()
        .await
        .as_ref()
        .map(|b| b.user_id.clone());
    let broadcaster = broadcaster.to_string();
    let callback = format!("{}/twitch_webhook", app.config.base_url);
    let mut complete = bot_id.is_some();
    for (kind, version, condition) in subscriptions(&broadcaster, bot_id.as_deref()) {
        let body = serde_json::json!({
            "type": kind,
            "version": version,
            "condition": condition,
            "transport": {"method": "webhook", "callback": callback, "secret": tw.eventsub_secret},
        });
        let response = app
            .http
            .post(format!(
                "{}/eventsub/subscriptions",
                app.twitch_endpoints.api
            ))
            .bearer_auth(&token)
            .header("Client-Id", &tw.client_id)
            .json(&body)
            .send()
            .await;
        match response {
            Ok(r) if r.status().is_success() || r.status() == StatusCode::CONFLICT => {}
            Ok(r) => {
                complete = false;
                let status = r.status();
                let detail: String = r
                    .text()
                    .await
                    .unwrap_or_default()
                    .chars()
                    .take(300)
                    .collect();
                tracing::warn!(
                    "[Twitch][EventSub][{}] {kind}: HTTP {status} {detail}",
                    ch.slug
                );
            }
            Err(_) => {
                complete = false;
                tracing::warn!("[Twitch][EventSub][{}] {kind}: error de red", ch.slug);
            }
        }
    }
    if complete {
        tracing::info!("[Twitch][EventSub][{}] Suscripciones listas", ch.slug);
    } else if bot_id.is_none() {
        tracing::warn!(
            "[Twitch][EventSub][{}] Falta autorizar la cuenta bot: el chat no llegará",
            ch.slug
        );
    }
    complete
}

#[cfg(test)]
mod tests {
    use super::*;

    fn signed(secret: &str, kind: &str, timestamp: &str, body: &str) -> HeaderMap {
        let mut mac = Hmac::<Sha256>::new_from_slice(secret.as_bytes()).unwrap();
        mac.update(b"msg-1");
        mac.update(timestamp.as_bytes());
        mac.update(body.as_bytes());
        let hex: String = mac
            .finalize()
            .into_bytes()
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect();
        let mut headers = HeaderMap::new();
        headers.insert("Twitch-Eventsub-Message-Id", "msg-1".parse().unwrap());
        headers.insert(
            "Twitch-Eventsub-Message-Timestamp",
            timestamp.parse().unwrap(),
        );
        headers.insert(
            "Twitch-Eventsub-Message-Signature",
            format!("sha256={hex}").parse().unwrap(),
        );
        headers.insert("Twitch-Eventsub-Message-Type", kind.parse().unwrap());
        headers
    }

    #[test]
    fn signed_deliveries_are_verified_and_tampering_or_old_ones_rejected() {
        let ts = "2026-11-15T12:00:00.123456789Z";
        let now = chrono::DateTime::parse_from_rfc3339(ts)
            .unwrap()
            .timestamp();
        let body = r#"{"subscription":{"type":"channel.follow"},"event":{}}"#;
        let headers = signed("secret-1234", "notification", ts, body);
        assert_eq!(
            verify("secret-1234", &headers, body.as_bytes(), now),
            Ok(Delivery::Notification {
                id: "msg-1".into(),
                kind: "channel.follow".into()
            })
        );
        assert_eq!(
            verify("other-secret", &headers, body.as_bytes(), now),
            Err(StatusCode::FORBIDDEN)
        );
        assert!(verify("secret-1234", &headers, b"{}", now).is_err());
        assert!(verify("secret-1234", &headers, body.as_bytes(), now + 601).is_err());
        assert!(verify("secret-1234", &headers, body.as_bytes(), now - 601).is_err());
        let mut bad = headers.clone();
        bad.insert(
            "Twitch-Eventsub-Message-Signature",
            "sha256=zz".parse().unwrap(),
        );
        assert!(verify("secret-1234", &bad, body.as_bytes(), now).is_err());
        bad.remove("Twitch-Eventsub-Message-Signature");
        assert!(verify("secret-1234", &bad, body.as_bytes(), now).is_err());
    }

    #[test]
    fn verification_returns_the_challenge_and_revocations_their_status() {
        let ts = "2026-11-15T12:00:00Z";
        let now = chrono::DateTime::parse_from_rfc3339(ts)
            .unwrap()
            .timestamp();
        let body =
            r#"{"challenge":"pogchamp-kappa-360noscope","subscription":{"type":"stream.online"}}"#;
        let headers = signed("s3cret-key", "webhook_callback_verification", ts, body);
        assert_eq!(
            verify("s3cret-key", &headers, body.as_bytes(), now),
            Ok(Delivery::Verification {
                challenge: "pogchamp-kappa-360noscope".into()
            })
        );
        let body =
            r#"{"subscription":{"type":"channel.chat.message","status":"authorization_revoked"}}"#;
        let headers = signed("s3cret-key", "revocation", ts, body);
        assert_eq!(
            verify("s3cret-key", &headers, body.as_bytes(), now),
            Ok(Delivery::Revocation {
                kind: "channel.chat.message".into(),
                status: "authorization_revoked".into()
            })
        );
    }

    #[test]
    fn chat_subscription_needs_the_bot_account() {
        let without = subscriptions("5", None);
        assert!(without
            .iter()
            .all(|(kind, ..)| *kind != "channel.chat.message"));
        assert_eq!(without.len(), 6);
        let with = subscriptions("5", Some("1609918794"));
        assert_eq!(with[0].0, "channel.chat.message");
        assert_eq!(with[0].2["user_id"], "1609918794");
        let follow = with
            .iter()
            .find(|(kind, ..)| *kind == "channel.follow")
            .unwrap();
        assert_eq!(
            (follow.1, &follow.2["moderator_user_id"]),
            ("2", &serde_json::json!("5"))
        );
    }
}
