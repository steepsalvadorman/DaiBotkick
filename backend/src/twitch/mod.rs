//! Integración con Twitch: conexión de canales (`auth`), cuenta bot DaiBotsito (`bot`) y
//! mantenimiento de tokens. Los eventos del chat llegan en la etapa 4 del plan.

pub mod auth;
pub mod bot;
pub mod eventsub;
pub mod ingest;

use crate::state::{AppState, ChannelState};
use std::sync::{atomic::Ordering, Arc};
use tokio::time::{sleep, Duration};

/// Producción por defecto; los tests de integración inyectan un servicio HTTP local.
pub struct Endpoints {
    pub oauth: String,
    pub api: String,
}

impl Default for Endpoints {
    fn default() -> Self {
        Self {
            oauth: "https://id.twitch.tv/oauth2".into(),
            api: "https://api.twitch.tv/helix".into(),
        }
    }
}

/// Credenciales de la app de Twitch (variables `TWITCH_*`).
pub struct Config {
    pub client_id: String,
    pub client_secret: String,
    /// Firma de los webhooks EventSub.
    pub eventsub_secret: String,
}

impl Config {
    /// `None` si falta alguna variable: Twitch queda desactivado y Kick funciona igual.
    pub fn from_env() -> Option<Self> {
        let var = |name: &str| std::env::var(name).ok().filter(|v| !v.trim().is_empty());
        Some(Self {
            client_id: var("TWITCH_CLIENT_ID")?,
            client_secret: var("TWITCH_CLIENT_SECRET")?,
            eventsub_secret: var("TWITCH_EVENTSUB_SECRET")?,
        })
    }
}

/// Usuario de Twitch (`GET /helix/users` con el token del propio usuario).
#[derive(Debug, Clone, PartialEq)]
pub struct User {
    pub id: String,
    pub login: String,
    pub display_name: String,
    pub profile_image_url: String,
}

/// Datos del usuario dueño de `token`.
pub async fn current_user(global: &AppState, token: &str) -> Result<User, String> {
    let Some(tw) = &global.config.twitch else {
        return Err("Twitch no está configurado".into());
    };
    let response = global
        .http
        .get(format!("{}/users", global.twitch_endpoints.api))
        .bearer_auth(token)
        .header("Client-Id", &tw.client_id)
        .send()
        .await
        .map_err(|_| "No se pudo conectar con Twitch".to_string())?;
    if !response.status().is_success() {
        return Err(format!(
            "Twitch rechazó la consulta ({})",
            response.status()
        ));
    }
    let json: serde_json::Value = response
        .json()
        .await
        .map_err(|_| "Twitch devolvió una respuesta inválida".to_string())?;
    parse_user(&json["data"][0]).ok_or_else(|| "Twitch no devolvió un usuario válido".into())
}

fn parse_user(data: &serde_json::Value) -> Option<User> {
    let id = data["id"].as_str().filter(|id| id.parse::<u64>().is_ok())?;
    let login = data["login"].as_str().filter(|l| valid_login(l))?;
    Some(User {
        id: id.to_owned(),
        login: login.to_owned(),
        display_name: data["display_name"].as_str().unwrap_or(login).to_owned(),
        profile_image_url: data["profile_image_url"]
            .as_str()
            .filter(|u| crate::state::avatar_url_valid(u))
            .unwrap_or("")
            .to_owned(),
    })
}

/// Logins de Twitch: 4–25 caracteres, minúsculas, dígitos y `_`.
pub fn valid_login(login: &str) -> bool {
    (1..=25).contains(&login.len())
        && login
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_')
}

/// Resultado de `GET /oauth2/validate`, que Twitch exige llamar cada hora por token.
#[derive(Debug, PartialEq)]
pub enum Validation {
    Valid,
    /// Revocado o vencido: hay que renovarlo.
    Invalid,
    /// Fallo de red o de Twitch: se reintenta más tarde sin tocar el token.
    Unknown,
}

pub async fn validate(global: &AppState, token: &str) -> Validation {
    let response = global
        .http
        .get(format!("{}/validate", global.twitch_endpoints.oauth))
        .header("Authorization", format!("OAuth {token}"))
        .send()
        .await;
    match response {
        Ok(r) if r.status().is_success() => Validation::Valid,
        Ok(r) if r.status() == reqwest::StatusCode::UNAUTHORIZED => Validation::Invalid,
        _ => Validation::Unknown,
    }
}

/// Crea las suscripciones EventSub del canal y mantiene su token: lo valida cada hora y lo
/// renueva antes de que venza o si Twitch lo invalida.
pub async fn run_channel(ch: Arc<ChannelState>, global: Arc<AppState>) {
    let subscriptions = async {
        sleep(Duration::from_secs(5)).await;
        let mut delay = 30;
        // Sin la cuenta bot falta el chat: se reintenta hasta que se autorice.
        while !eventsub::subscribe(&ch, &global).await {
            sleep(Duration::from_secs(delay)).await;
            delay = (delay * 2).min(300);
        }
    };
    tokio::join!(subscriptions, maintain_token(&ch, &global));
}

async fn maintain_token(ch: &Arc<ChannelState>, global: &Arc<AppState>) {
    loop {
        let now = chrono::Utc::now().timestamp().max(0) as u64;
        let remaining = ch.token_expires.load(Ordering::Relaxed).saturating_sub(now);
        let token = ch.access_token.read().await.clone();
        let needs_refresh =
            remaining < 600 || validate(global, &token).await == Validation::Invalid;
        if needs_refresh && !crate::oauth::refresh_if_current(ch, global, &token).await {
            tracing::warn!(
                "[Twitch][{}] No se pudo renovar el token; el streamer debe volver a conectar",
                ch.slug
            );
        }
        let remaining = ch
            .token_expires
            .load(Ordering::Relaxed)
            .saturating_sub(chrono::Utc::now().timestamp().max(0) as u64);
        sleep(Duration::from_secs(
            remaining.saturating_sub(600).clamp(60, 3600),
        ))
        .await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn users_need_a_numeric_id_and_a_valid_login() {
        let user = parse_user(&serde_json::json!({
            "id": "701010126", "login": "sr_dai_twitch", "display_name": "Sr_Dai_Twitch",
            "profile_image_url": "https://static-cdn.jtvnw.net/a.png"
        }))
        .unwrap();
        assert_eq!(user.login, "sr_dai_twitch");
        assert_eq!(user.display_name, "Sr_Dai_Twitch");
        assert_eq!(user.profile_image_url, "https://static-cdn.jtvnw.net/a.png");
        let unsafe_avatar = parse_user(&serde_json::json!({
            "id": "1", "login": "ana", "profile_image_url": "javascript:alert(1)"
        }))
        .unwrap();
        assert_eq!(unsafe_avatar.display_name, "ana");
        assert_eq!(unsafe_avatar.profile_image_url, "");
        for bad in [
            serde_json::json!({"id": "x1", "login": "ana"}),
            serde_json::json!({"id": "1", "login": "Ana"}),
            serde_json::json!({"id": "1", "login": "a-b"}),
            serde_json::json!({"id": "1"}),
        ] {
            assert!(parse_user(&bad).is_none(), "{bad}");
        }
    }

    #[test]
    fn logins_are_lowercase_twitch_names() {
        assert!(valid_login("daibotsito"));
        assert!(valid_login("sr_dai_twitch"));
        assert!(!valid_login(""));
        assert!(!valid_login("twitch:pepe"));
        assert!(!valid_login(&"a".repeat(26)));
    }
}
