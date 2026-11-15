//! Cuenta bot de Twitch (DaiBotsito): escribe en el chat de todos los canales de Twitch.
//! Se autoriza una sola vez en `/auth/twitch/bot`; su token se guarda en `bot_accounts`
//! y se valida cada hora y se renueva aquí.

use crate::{db, platform::Platform, state::AppState};
use std::sync::Arc;
use tokio::time::{sleep, Duration};

/// ID de Twitch de la cuenta `daibotsito`. Solo esa cuenta puede autorizarse como bot.
pub const BOT_USER_ID: &str = "1609918794";

/// Permisos de la cuenta bot: leer y escribir el chat como bot verificado por el canal.
pub const SCOPES: &str = "user:read:chat user:write:chat user:bot";

#[derive(Clone, Debug)]
pub struct BotAccount {
    pub user_id: String,
    pub login: String,
    pub access_token: String,
    pub refresh_token: String,
    /// Unix (segundos).
    pub token_expires: i64,
}

/// Carga la cuenta guardada al arrancar; `None` si todavía no se autorizó.
pub async fn load(global: &AppState) {
    match db::load_bot_account(&global.db, Platform::Twitch).await {
        Ok(Some(bot)) => {
            tracing::info!("[Twitch][bot] Cuenta {} cargada", bot.login);
            *global.twitch_bot.write().await = Some(bot);
        }
        Ok(None) => tracing::warn!(
            "[Twitch][bot] Sin autorizar: abre /auth/twitch/bot con la cuenta daibotsito"
        ),
        Err(e) => tracing::error!("[Twitch][bot] No se pudo cargar: {e}"),
    }
}

/// Valida el token del bot cada hora y lo renueva antes de que venza o si Twitch lo invalida.
pub async fn run(global: Arc<AppState>) {
    loop {
        let bot = global.twitch_bot.read().await.clone();
        let mut wait = 3600;
        if let Some(bot) = bot {
            let remaining = bot.token_expires - chrono::Utc::now().timestamp();
            if remaining < 600
                || super::validate(&global, &bot.access_token).await == super::Validation::Invalid
            {
                match refresh(&global, &bot).await {
                    Ok(next) => {
                        wait = (next.token_expires - chrono::Utc::now().timestamp() - 600)
                            .clamp(60, 3600) as u64;
                        *global.twitch_bot.write().await = Some(next);
                    }
                    Err(e) => {
                        tracing::warn!("[Twitch][bot] {e}; vuelve a autorizar /auth/twitch/bot");
                        wait = 300;
                    }
                }
            } else {
                wait = (remaining - 600).clamp(60, 3600) as u64;
            }
        }
        tokio::select! {
            _ = global.shutdown.cancelled() => return,
            _ = sleep(Duration::from_secs(wait)) => {}
        }
    }
}

async fn refresh(global: &AppState, bot: &BotAccount) -> Result<BotAccount, String> {
    let (url, client_id, client_secret) = crate::oauth::token_endpoint(Platform::Twitch, global)
        .ok_or("Twitch no está configurado")?;
    let response = global
        .http
        .post(url)
        .form(&[
            ("grant_type", "refresh_token"),
            ("refresh_token", bot.refresh_token.as_str()),
            ("client_id", client_id),
            ("client_secret", client_secret),
        ])
        .send()
        .await
        .map_err(|_| "Error de red al renovar el token".to_string())?;
    if !response.status().is_success() {
        return Err(format!(
            "Twitch rechazó la renovación ({})",
            response.status()
        ));
    }
    let tokens = super::auth::Tokens::from_json(
        &response
            .json()
            .await
            .map_err(|_| "Respuesta de token inválida".to_string())?,
    )
    .ok_or("Twitch no devolvió un token válido")?;
    let next = BotAccount {
        access_token: tokens.access_token,
        refresh_token: if tokens.refresh_token.is_empty() {
            bot.refresh_token.clone()
        } else {
            tokens.refresh_token
        },
        token_expires: tokens.expires_at,
        ..bot.clone()
    };
    db::save_bot_account(&global.db, Platform::Twitch, &next, SCOPES)
        .await
        .map_err(|e| format!("No se pudo guardar el token: {e}"))?;
    Ok(next)
}
