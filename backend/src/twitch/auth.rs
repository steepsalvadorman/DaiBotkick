//! Conexión con Twitch (código de autorización de OAuth):
//! - `/auth/twitch`: un streamer conecta su canal y recibe su overlay V-Saturn y sus docks.
//! - `/auth/twitch/bot`: se autoriza una vez la cuenta bot DaiBotsito (solo esa cuenta).

use super::bot::{self, BotAccount};
use crate::{
    auth::{error_page, escape_html, matches_cookie, oauth_cookie, url_encode},
    channel, db,
    platform::Platform,
    state::AppState,
};
use axum::{
    extract::{Query, State},
    http::{header, HeaderMap},
    response::{Html, IntoResponse, Redirect, Response},
};
use std::{collections::HashMap, sync::Arc};

/// Permisos del streamer: que DaiBotsito lea y escriba en su chat, alertas de follows y
/// suscripciones, y cambiar título y categoría desde el dock.
const CHANNEL_SCOPES: &str =
    "channel:bot moderator:read:followers channel:read:subscriptions channel:manage:broadcast";

/// Marca guardada con cada `state` para que un `state` de un flujo no sirva en el otro.
const CHANNEL_FLOW: &str = "twitch";
const BOT_FLOW: &str = "twitch-bot";

/// Tokens de `POST /oauth2/token` (código o renovación).
pub struct Tokens {
    pub access_token: String,
    pub refresh_token: String,
    /// Unix (segundos).
    pub expires_at: i64,
}

impl Tokens {
    pub fn from_json(json: &serde_json::Value) -> Option<Self> {
        let access_token = json["access_token"].as_str().filter(|t| !t.is_empty())?;
        Some(Self {
            access_token: access_token.to_owned(),
            refresh_token: json["refresh_token"].as_str().unwrap_or("").to_owned(),
            expires_at: chrono::Utc::now().timestamp()
                + json["expires_in"].as_i64().unwrap_or(3600).max(0),
        })
    }
}

pub async fn redirect_channel(State(app): State<Arc<AppState>>) -> Response {
    authorize(&app, CHANNEL_FLOW, "/auth/twitch/callback", CHANNEL_SCOPES).await
}

pub async fn redirect_bot(State(app): State<Arc<AppState>>) -> Response {
    authorize(&app, BOT_FLOW, "/auth/twitch/bot/callback", bot::SCOPES).await
}

async fn authorize(app: &AppState, flow: &str, callback: &str, scopes: &str) -> Response {
    let Some(tw) = &app.config.twitch else {
        return error_page("Twitch todavía no está configurado en este servidor.").into_response();
    };
    let state = uuid::Uuid::new_v4().to_string();
    if db::save_oauth_state(&app.db, &state, flow).await.is_err() {
        return error_page("No se pudo iniciar la conexión con Twitch.").into_response();
    }
    // `force_verify` muestra siempre la pantalla de Twitch, para elegir con qué cuenta entrar.
    let url = format!(
        "{oauth}/authorize?client_id={client}&redirect_uri={redirect}&response_type=code&scope={scope}&state={state}&force_verify=true",
        oauth = app.twitch_endpoints.oauth,
        client = url_encode(&tw.client_id),
        redirect = url_encode(&format!("{}{callback}", app.config.base_url)),
        scope = url_encode(scopes),
    );
    (
        [(header::SET_COOKIE, oauth_cookie(app, &state))],
        Redirect::temporary(&url),
    )
        .into_response()
}

/// Comprueba `state` y cookie, y cambia el código por tokens. Devuelve el mensaje de error.
async fn exchange(
    app: &AppState,
    headers: &HeaderMap,
    params: &HashMap<String, String>,
    flow: &str,
    callback: &str,
) -> Result<Tokens, &'static str> {
    if params.contains_key("error") {
        return Err("Cancelaste la autorización en Twitch.");
    }
    let (Some(code), Some(state)) = (params.get("code"), params.get("state")) else {
        return Err("Twitch no devolvió el código de autorización.");
    };
    if !matches_cookie(headers, state) {
        return Err("La sesión no coincide. Vuelve a empezar desde este navegador.");
    }
    match db::consume_oauth_state(&app.db, state).await {
        Ok(Some(saved)) if saved == flow => {}
        Ok(_) => return Err("El enlace de autorización venció. Vuelve a empezar."),
        Err(_) => return Err("Base de datos no disponible."),
    }
    let tw = app
        .config
        .twitch
        .as_ref()
        .ok_or("Twitch no está configurado.")?;
    let redirect_uri = format!("{}{callback}", app.config.base_url);
    let response = app
        .http
        .post(format!("{}/token", app.twitch_endpoints.oauth))
        .form(&[
            ("client_id", tw.client_id.as_str()),
            ("client_secret", tw.client_secret.as_str()),
            ("code", code.as_str()),
            ("grant_type", "authorization_code"),
            ("redirect_uri", redirect_uri.as_str()),
        ])
        .send()
        .await
        .map_err(|_| "No se pudo conectar con Twitch.")?;
    if !response.status().is_success() {
        tracing::warn!("[Twitch][OAuth] Código rechazado: {}", response.status());
        return Err("Twitch rechazó la autorización. Vuelve a intentarlo.");
    }
    let json = response
        .json()
        .await
        .map_err(|_| "Twitch devolvió una respuesta inválida.")?;
    Tokens::from_json(&json).ok_or("Twitch no devolvió un token válido.")
}

pub async fn channel_callback(
    State(app): State<Arc<AppState>>,
    headers: HeaderMap,
    Query(params): Query<HashMap<String, String>>,
) -> Html<String> {
    match connect_channel(&app, &headers, &params).await {
        Ok(page) => Html(page),
        Err(message) => error_page(&message),
    }
}

async fn connect_channel(
    app: &Arc<AppState>,
    headers: &HeaderMap,
    params: &HashMap<String, String>,
) -> Result<String, String> {
    let tokens = exchange(app, headers, params, CHANNEL_FLOW, "/auth/twitch/callback").await?;
    let user = super::current_user(app, &tokens.access_token).await?;
    if user.id == bot::BOT_USER_ID {
        return Err("Esa es la cuenta del bot. Conecta la cuenta de tu canal.".into());
    }
    let Ok(broadcaster_id) = user.id.parse::<i64>() else {
        return Err("Twitch devolvió un ID de usuario inválido.".into());
    };
    // Si el streamer cambió su nombre en Twitch, su canal se renombra y conserva tokens y ajustes.
    let renamed =
        db::rename_channel_by_broadcaster(&app.db, Platform::Twitch, broadcaster_id, &user.login)
            .await
            .map_err(|e| {
                tracing::error!("[Twitch][Auth] No se pudo renombrar el canal: {e}");
                "No se pudo actualizar tu canal.".to_string()
            })?;
    if let Some(old) = renamed {
        if let Some((_, previous)) = app.channels.remove(&Platform::Twitch.channel_key(&old)) {
            previous.cancel.cancel();
        }
        tracing::info!("[Twitch][Auth] Canal renombrado: {old} → {}", user.login);
    }
    let key = Platform::Twitch.channel_key(&user.login);
    let existing_panel = app
        .channels
        .get(&key)
        .map(|c| c.panel_token.clone())
        .unwrap_or_default();
    let row = db::ChannelRow {
        platform: Platform::Twitch.as_str().into(),
        slug: user.login.clone(),
        broadcaster_user_id: Some(broadcaster_id),
        chatroom_id: None,
        access_token: tokens.access_token,
        refresh_token: tokens.refresh_token,
        token_expires: tokens.expires_at,
        panel_token: if existing_panel.is_empty() {
            uuid::Uuid::new_v4().to_string()
        } else {
            existing_panel
        },
        cmd_discord: String::new(),
        cmd_redes: String::new(),
        cmd_pc: String::new(),
        cmd_horario: String::new(),
        follow_goal: 100,
        queue_state: r#"{"items":[],"version":0}"#.into(),
        show_video: true,
        playback_token: uuid::Uuid::new_v4().to_string(),
    };
    db::upsert_channel(&app.db, &row).await.map_err(|e| {
        tracing::error!("[Twitch][Auth] No se pudo guardar el canal: {e}");
        "No se pudo guardar tu canal.".to_string()
    })?;
    let row = db::load_channel(&app.db, Platform::Twitch, &user.login)
        .await
        .map_err(|_| "No se pudo cargar tu canal.".to_string())?;
    let (playback_token, panel_token) = (row.playback_token.clone(), row.panel_token.clone());
    channel::start_channel(row, app.clone())
        .await
        .map_err(|_| "No se pudo iniciar tu canal.".to_string())?;
    if let Some(ch) = app.channels.get(&key) {
        *ch.display_name.write().await = user.display_name.clone();
        *ch.avatar_url.write().await = user.profile_image_url.clone();
    }
    tracing::info!("[Twitch][Auth] Canal conectado: {}", user.login);
    let base = escape_html(&app.config.base_url);
    let login = &user.login;
    Ok(crate::auth::connected_page(
        Platform::Twitch,
        login,
        &format!("{base}/vsaturn.html?tw={login}#token={playback_token}"),
        &format!("{base}/dock-chat.html?tw={login}#token={panel_token}"),
        &format!("{base}/dock-stream.html?tw={login}#token={panel_token}"),
    ))
}

pub async fn bot_callback(
    State(app): State<Arc<AppState>>,
    headers: HeaderMap,
    Query(params): Query<HashMap<String, String>>,
) -> Html<String> {
    match connect_bot(&app, &headers, &params).await {
        Ok(login) => Html(format!(
            r#"<!doctype html><html lang="es"><head><meta charset="utf-8"><meta name="viewport" content="width=device-width, initial-scale=1"><title>DaiBotsito autorizado</title></head>
<body style="font-family:system-ui,sans-serif;background:#07020f;color:#ece8f4;padding:40px">
<h1 style="color:#a970ff">✓ Cuenta bot autorizada</h1>
<p><b>{}</b> ya puede escribir en el chat de los canales de Twitch conectados.</p>
<p>No hace falta repetir este paso: el token se renueva solo.</p></body></html>"#,
            escape_html(&login)
        )),
        Err(message) => error_page(&message),
    }
}

async fn connect_bot(
    app: &AppState,
    headers: &HeaderMap,
    params: &HashMap<String, String>,
) -> Result<String, String> {
    let tokens = exchange(app, headers, params, BOT_FLOW, "/auth/twitch/bot/callback").await?;
    let user = super::current_user(app, &tokens.access_token).await?;
    if user.id != bot::BOT_USER_ID {
        tracing::warn!("[Twitch][bot] Autorización rechazada para {}", user.login);
        return Err(format!(
            "Entraste como {}. Cierra sesión en Twitch y autoriza con la cuenta daibotsito.",
            user.login
        ));
    }
    let account = BotAccount {
        user_id: user.id,
        login: user.login.clone(),
        access_token: tokens.access_token,
        refresh_token: tokens.refresh_token,
        token_expires: tokens.expires_at,
    };
    db::save_bot_account(&app.db, Platform::Twitch, &account, bot::SCOPES)
        .await
        .map_err(|e| {
            tracing::error!("[Twitch][bot] No se pudo guardar: {e}");
            "No se pudo guardar la cuenta del bot.".to_string()
        })?;
    *app.twitch_bot.write().await = Some(account);
    tracing::info!("[Twitch][bot] Cuenta {} autorizada", user.login);
    Ok(user.login)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tokens_need_an_access_token_and_compute_expiry() {
        let now = chrono::Utc::now().timestamp();
        let t = Tokens::from_json(
            &serde_json::json!({"access_token":"a","refresh_token":"r","expires_in":100}),
        )
        .unwrap();
        assert_eq!(
            (t.access_token.as_str(), t.refresh_token.as_str()),
            ("a", "r")
        );
        assert!((now + 100..=now + 101).contains(&t.expires_at));
        assert!(Tokens::from_json(&serde_json::json!({"access_token":""})).is_none());
        assert!(Tokens::from_json(&serde_json::json!({})).is_none());
    }
}
