use axum::{
    extract::{Query, State},
    http::{header, HeaderMap, StatusCode},
    response::{Html, IntoResponse, Redirect, Response},
};
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine as _};
use rand::RngCore;
use sha2::{Digest, Sha256};
use std::{collections::HashMap, sync::Arc};
use tracing::{info, warn};

use crate::{channel, db, state::AppState};

pub async fn start_oauth() -> Html<String> {
    Html(
        r#"<!DOCTYPE html>
<html lang="es">
<head>
  <meta charset="UTF-8">
  <title>DaiBot — Conectar canal</title>
  <style>
    body{font-family:monospace;background:#060010;color:#53FC18;display:flex;
          align-items:center;justify-content:center;min-height:100vh;margin:0}
    .card{border:2px solid #53FC18;padding:40px;max-width:480px;text-align:center;
           box-shadow:0 0 32px #53FC1844}
    h1{font-size:2em;margin:0 0 8px}
    p{color:#aaa;margin:16px 0 32px}
    a.btn{background:#53FC18;color:#060010;padding:14px 32px;text-decoration:none;
            font-weight:bold;font-size:1.1em;display:inline-block}
    a.btn:hover{opacity:.85}
  </style>
</head>
<body>
  <div class="card">
    <h1>[ D A I B O T ]</h1>
    <p>Conecta tu canal de Kick para usar el bot, overlay y comandos de chat.</p>
    <a class="btn" href="/auth/kick">Conectar con Kick</a>
  </div>
</body>
</html>"#
            .to_string(),
    )
}

pub async fn redirect_to_kick(State(state): State<Arc<AppState>>) -> Response {
    let mut verifier_bytes = [0u8; 32];
    rand::thread_rng().fill_bytes(&mut verifier_bytes);
    let code_verifier = URL_SAFE_NO_PAD.encode(verifier_bytes);

    let mut hasher = Sha256::new();
    hasher.update(code_verifier.as_bytes());
    let code_challenge = URL_SAFE_NO_PAD.encode(hasher.finalize());

    let state_token = uuid::Uuid::new_v4().to_string();
    if db::save_oauth_state(&state.db, &state_token, &code_verifier)
        .await
        .is_err()
    {
        return (StatusCode::SERVICE_UNAVAILABLE, "No se pudo iniciar OAuth").into_response();
    }

    let redirect_uri = format!("{}/auth/callback", state.config.base_url);
    let scopes = "user:read channel:read chat:write events:subscribe";

    let url = format!(
        "{oauth}/authorize\
         ?client_id={client_id}\
         &redirect_uri={redir}\
         &response_type=code\
         &scope={scope}\
         &state={state}\
         &code_challenge={challenge}\
         &code_challenge_method=S256",
        client_id = state.config.client_id,
        redir = url_encode(&redirect_uri),
        scope = url_encode(scopes),
        state = state_token,
        challenge = code_challenge,
        oauth = state.kick_endpoints.oauth,
    );
    let secure = if state.config.base_url.starts_with("https://") {
        "; Secure"
    } else {
        ""
    };
    let cookie = format!(
        "daibot_oauth={state_token}; HttpOnly; SameSite=Lax; Path=/auth; Max-Age=600{secure}"
    );
    ([(header::SET_COOKIE, cookie)], Redirect::temporary(&url)).into_response()
}

pub async fn handle_callback(
    State(st): State<Arc<AppState>>,
    headers: HeaderMap,
    Query(params): Query<HashMap<String, String>>,
) -> Html<String> {
    let err_page = |msg: &str| {
        Html(format!(
            r#"<html><body style="font-family:monospace;background:#060010;color:#f55;padding:40px">
           <h1>Error</h1><p>{msg}</p><a href="/" style="color:#53FC18">← Volver</a></body></html>"#
        ))
    };

    let code = match params.get("code") {
        Some(c) => c.clone(),
        None => return err_page("No se recibió código de autorización"),
    };
    let state_tok = match params.get("state") {
        Some(s) => s.clone(),
        None => return err_page("No se recibió state"),
    };
    if !matches_cookie(&headers, &state_tok) {
        return err_page("La sesion OAuth no coincide. Vuelve a conectar tu canal.");
    }
    let code_verifier = match db::consume_oauth_state(&st.db, &state_tok).await {
        Ok(Some(v)) => v,
        Ok(None) => return err_page("State invalido o expirado"),
        Err(_) => return err_page("Base de datos no disponible"),
    };

    // Intercambiar código por tokens
    let redirect_uri = format!("{}/auth/callback", st.config.base_url);
    let token_resp = st
        .http
        .post(format!("{}/token", st.kick_endpoints.oauth))
        .form(&[
            ("grant_type", "authorization_code"),
            ("client_id", &st.config.client_id),
            ("client_secret", &st.config.client_secret),
            ("code", &code),
            ("redirect_uri", &redirect_uri),
            ("code_verifier", &code_verifier),
        ])
        .send()
        .await;

    let token_json: serde_json::Value = match token_resp {
        Ok(r) if r.status().is_success() => r.json().await.unwrap_or_default(),
        Ok(_) => return err_page("Kick rechazo la solicitud"),
        Err(e) => {
            warn!("[OAuth] Token exchange error: {e}");
            return err_page("Error al obtener tokens");
        }
    };

    let access_token = match token_json["access_token"].as_str() {
        Some(t) if !t.is_empty() => t.to_string(),
        Some(_) => return err_page("Token vacio"),
        None => return err_page("No se recibio un token valido"),
    };
    let refresh_token = token_json["refresh_token"]
        .as_str()
        .unwrap_or("")
        .to_string();
    let expires_in = token_json["expires_in"].as_u64().unwrap_or(7200);
    let token_expires = (std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
        + expires_in) as i64;

    // Obtener info del canal
    let chan_json: serde_json::Value = match st
        .http
        .get(format!("{}/channels", st.kick_endpoints.api))
        .header("Authorization", format!("Bearer {access_token}"))
        .send()
        .await
    {
        Ok(r) if r.status().is_success() => r.json().await.unwrap_or_default(),
        Ok(_) => return err_page("Kick rechazó la consulta del canal"),
        Err(e) => {
            warn!("[Auth] Error obteniendo info del canal: {e}");
            return err_page("Error obteniendo info del canal");
        }
    };

    let ch = chan_json["data"]
        .as_array()
        .and_then(|a| a.first())
        .cloned()
        .unwrap_or_default();

    let Some(slug) = ch["slug"]
        .as_str()
        .or_else(|| ch["username"].as_str())
        .map(str::to_lowercase)
        .filter(|s| valid_slug(s))
    else {
        return err_page("Identidad de canal invalida");
    };
    let Some(broadcaster_id) = ch["broadcaster_user_id"].as_i64().filter(|id| *id > 0) else {
        return err_page("Canal sin identificador valido");
    };
    let chatroom_id = ch["chatroom"]["id"]
        .as_i64()
        .or_else(|| ch["chatroom_id"].as_i64())
        .unwrap_or(broadcaster_id);

    // Panel token: generar si es nuevo canal, conservar si ya existe
    let existing_panel = st
        .channels
        .get(&slug)
        .map(|c| c.panel_token.clone())
        .unwrap_or_default();
    let panel_token = if existing_panel.is_empty() {
        uuid::Uuid::new_v4().to_string()
    } else {
        existing_panel
    };

    let row = db::ChannelRow {
        slug: slug.clone(),
        broadcaster_user_id: Some(broadcaster_id),
        chatroom_id: Some(chatroom_id),
        access_token,
        refresh_token,
        token_expires,
        panel_token: panel_token.clone(),
        cmd_discord: String::new(),
        cmd_redes: String::new(),
        cmd_pc: String::new(),
        cmd_horario: String::new(),
        follow_goal: 100,
        queue_state: String::from("{\"items\":[],\"version\":0}"),
        show_video: true,
        playback_token: uuid::Uuid::new_v4().to_string(),
    };

    if let Err(e) = db::upsert_channel(&st.db, &row).await {
        warn!("[DB] Registro fallo: {e}");
        return err_page("No se pudo guardar el canal");
    }
    let row = match db::load_channel(&st.db, &slug).await {
        Ok(row) => row,
        Err(_) => return err_page("No se pudo cargar el canal"),
    };
    let playback_token = row.playback_token.clone();
    if channel::start_channel(row, st.clone()).await.is_err() {
        return err_page("No se pudo iniciar el canal");
    }

    info!("[Auth] Canal registrado: {slug}");

    let base = escape_html(&st.config.base_url);
    Html(format!(
        r#"<!DOCTYPE html>
<html lang="es">
<head>
  <meta charset="UTF-8">
  <title>DaiBot — Conectado</title>
  <style>
    body{{font-family:monospace;background:#060010;color:#53FC18;padding:40px;max-width:600px;margin:auto}}
    h1{{color:#53FC18}} code{{background:#111;padding:8px 16px;display:block;margin:8px 0;word-break:break-all}}
    .dim{{color:#888}} a{{color:#53FC18}}
  </style>
</head>
<body>
  <h1>✅ ¡DaiBot conectado para {slug}!</h1>

  <h3>Overlay para OBS (Browser Source):</h3>
  <code>{base}/pixel.html?ch={slug}#token={playback_token}</code>
  <p>Esta URL permite controlar la reproduccion. Guardala solo en tu OBS y no la compartas.</p>

</body>
</html>"#
    ))
}

fn url_encode(s: &str) -> String {
    url::form_urlencoded::byte_serialize(s.as_bytes()).collect()
}

fn valid_slug(slug: &str) -> bool {
    !slug.is_empty()
        && slug.len() <= 100
        && slug
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
}
fn matches_cookie(headers: &HeaderMap, state: &str) -> bool {
    headers
        .get(header::COOKIE)
        .and_then(|h| h.to_str().ok())
        .is_some_and(|cookies| {
            cookies
                .split(';')
                .any(|cookie| cookie.trim().strip_prefix("daibot_oauth=") == Some(state))
        })
}
fn escape_html(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#39;")
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn callback_requires_matching_browser_cookie() {
        let mut headers = HeaderMap::new();
        assert!(!matches_cookie(&headers, "valid"));
        headers.insert(
            header::COOKIE,
            "other=x; daibot_oauth=valid".parse().unwrap(),
        );
        assert!(matches_cookie(&headers, "valid"));
        assert!(!matches_cookie(&headers, "other"));
    }
    #[test]
    fn external_channel_names_are_validated() {
        assert!(valid_slug("senior-dai_1"));
        assert!(!valid_slug("<script>"));
        assert!(!valid_slug(""));
    }
}
