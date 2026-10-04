use crate::state::AppState;
use axum::{
    extract::{Query, State},
    http::{header, HeaderMap, StatusCode},
    response::{IntoResponse, Redirect, Response},
};
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use rand::RngCore;
use sha2::{Digest, Sha256};
use std::{
    collections::HashMap,
    sync::Arc,
    time::{Duration, Instant},
};
use tokio::sync::Mutex;

const SESSION_SECONDS: u64 = 8 * 60 * 60;
const MAX_ENTRIES: usize = 10_000;

struct Pending {
    verifier: String,
    channel: String,
    token: String,
    expires: Instant,
}

struct Session {
    username: String,
    expires: Instant,
}

#[derive(Default)]
pub struct ViewerAuth {
    pending: Mutex<HashMap<String, Pending>>,
    sessions: Mutex<HashMap<String, Session>>,
}

fn cookie<'a>(headers: &'a HeaderMap, name: &str) -> Option<&'a str> {
    headers
        .get(header::COOKIE)?
        .to_str()
        .ok()?
        .split(';')
        .filter_map(|part| part.trim().split_once('='))
        .find_map(|(key, value)| (key == name).then_some(value))
}

impl ViewerAuth {
    pub async fn username(&self, headers: &HeaderMap) -> Option<String> {
        let id = cookie(headers, "bingo_session")?;
        let mut sessions = self.sessions.lock().await;
        sessions.retain(|_, session| session.expires > Instant::now());
        sessions.get(id).map(|session| session.username.clone())
    }

    async fn create_session(
        &self,
        username: String,
        headers: &HeaderMap,
    ) -> Result<String, &'static str> {
        let mut sessions = self.sessions.lock().await;
        sessions.retain(|_, session| session.expires > Instant::now());
        if let Some(old) = cookie(headers, "bingo_session") {
            sessions.remove(old);
        }
        if sessions.len() >= MAX_ENTRIES {
            return Err("Demasiadas sesiones. Intenta más tarde.");
        }
        let id = uuid::Uuid::new_v4().simple().to_string();
        sessions.insert(
            id.clone(),
            Session {
                username,
                expires: Instant::now() + Duration::from_secs(SESSION_SECONDS),
            },
        );
        Ok(id)
    }
}

#[derive(serde::Deserialize)]
pub struct LoginQuery {
    ch: String,
    token: String,
}

fn secure(app: &AppState) -> &'static str {
    if app.config.base_url.starts_with("https://") {
        "; Secure"
    } else {
        ""
    }
}

pub async fn login(State(app): State<Arc<AppState>>, Query(query): Query<LoginQuery>) -> Response {
    let valid = !query.ch.is_empty()
        && query.ch.len() <= 100
        && query
            .ch
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
        && query.token.len() == 32
        && query.token.bytes().all(|b| b.is_ascii_hexdigit());
    if !valid {
        return (
            StatusCode::BAD_REQUEST,
            "Enlace inválido. Solicita !carton en el chat.",
        )
            .into_response();
    }
    let mut bytes = [0u8; 32];
    rand::thread_rng().fill_bytes(&mut bytes);
    let verifier = URL_SAFE_NO_PAD.encode(bytes);
    let challenge = URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()));
    let state = uuid::Uuid::new_v4().simple().to_string();
    let mut pending = app.bingo_auth.pending.lock().await;
    pending.retain(|_, value| value.expires > Instant::now());
    if pending.len() >= MAX_ENTRIES {
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            "Demasiados inicios de sesión. Intenta más tarde.",
        )
            .into_response();
    }
    pending.insert(
        state.clone(),
        Pending {
            verifier,
            channel: query.ch,
            token: query.token,
            expires: Instant::now() + Duration::from_secs(600),
        },
    );
    drop(pending);
    let params = url::form_urlencoded::Serializer::new(String::new())
        .append_pair("client_id", &app.config.client_id)
        .append_pair(
            "redirect_uri",
            &format!(
                "{}/auth/bingo/callback",
                app.config.base_url.trim_end_matches('/')
            ),
        )
        .append_pair("response_type", "code")
        .append_pair("scope", "user:read")
        .append_pair("state", &state)
        .append_pair("code_challenge", &challenge)
        .append_pair("code_challenge_method", "S256")
        .finish();
    let cookie = format!(
        "bingo_oauth={state}; HttpOnly; SameSite=Lax; Path=/auth/bingo; Max-Age=600{}",
        secure(&app)
    );
    (
        [(header::SET_COOKIE, cookie)],
        Redirect::temporary(&format!("{}/authorize?{params}", app.kick_endpoints.oauth)),
    )
        .into_response()
}

async fn authenticate(app: &AppState, code: &str, verifier: &str) -> Result<String, String> {
    let redirect_uri = format!(
        "{}/auth/bingo/callback",
        app.config.base_url.trim_end_matches('/')
    );
    let response = app
        .http
        .post(format!("{}/token", app.kick_endpoints.oauth))
        .form(&[
            ("grant_type", "authorization_code"),
            ("client_id", &app.config.client_id),
            ("client_secret", &app.config.client_secret),
            ("code", code),
            ("redirect_uri", &redirect_uri),
            ("code_verifier", verifier),
        ])
        .send()
        .await
        .map_err(|_| "No se pudo conectar con Kick".to_string())?;
    if !response.status().is_success() {
        return Err(format!(
            "Kick rechazó el inicio de sesión ({})",
            response.status()
        ));
    }
    let tokens: serde_json::Value = response
        .json()
        .await
        .map_err(|_| "Kick devolvió una respuesta inválida".to_string())?;
    let access_token = tokens["access_token"]
        .as_str()
        .filter(|token| !token.is_empty())
        .ok_or("Kick no devolvió un token válido")?;
    let response = app
        .http
        .get(format!("{}/users", app.kick_endpoints.api))
        .bearer_auth(access_token)
        .send()
        .await
        .map_err(|_| "No se pudo verificar tu cuenta de Kick".to_string())?;
    if !response.status().is_success() {
        return Err(format!(
            "Kick rechazó la consulta de identidad ({})",
            response.status()
        ));
    }
    let users: serde_json::Value = response
        .json()
        .await
        .map_err(|_| "Kick devolvió una identidad inválida".to_string())?;
    let data = users["data"]
        .as_array()
        .filter(|data| data.len() == 1)
        .ok_or("No se pudo identificar tu cuenta")?;
    if data[0]["user_id"].as_u64().filter(|id| *id > 0).is_none() {
        return Err("Cuenta de Kick sin identificador válido".into());
    }
    data[0]["name"]
        .as_str()
        .filter(|name| !name.trim().is_empty())
        .map(str::to_lowercase)
        .ok_or("Cuenta de Kick sin nombre válido".into())
}

pub async fn callback(
    State(app): State<Arc<AppState>>,
    headers: HeaderMap,
    Query(params): Query<HashMap<String, String>>,
) -> Response {
    let Some(state) = params
        .get("state")
        .filter(|state| cookie(&headers, "bingo_oauth") == Some(state.as_str()))
    else {
        return (
            StatusCode::BAD_REQUEST,
            "La sesión OAuth no coincide. Vuelve al cartón e inicia sesión.",
        )
            .into_response();
    };
    let pending = app.bingo_auth.pending.lock().await.remove(state);
    let Some(pending) = pending.filter(|value| value.expires > Instant::now()) else {
        return (
            StatusCode::BAD_REQUEST,
            "Inicio de sesión expirado o ya utilizado. Vuelve al cartón.",
        )
            .into_response();
    };
    let Some(code) = params.get("code").filter(|code| !code.is_empty()) else {
        return (
            StatusCode::BAD_REQUEST,
            "Autorización cancelada. Vuelve al cartón para intentar de nuevo.",
        )
            .into_response();
    };
    let username = match authenticate(&app, code, &pending.verifier).await {
        Ok(username) => username,
        Err(message) => {
            tracing::warn!("[Bingo OAuth] {message}");
            return (StatusCode::BAD_GATEWAY, message).into_response();
        }
    };
    let session = match app.bingo_auth.create_session(username, &headers).await {
        Ok(session) => session,
        Err(message) => return (StatusCode::SERVICE_UNAVAILABLE, message).into_response(),
    };
    let fragment = url::form_urlencoded::Serializer::new(String::new())
        .append_pair("ch", &pending.channel)
        .append_pair("token", &pending.token)
        .finish();
    let mut response = Redirect::to(&format!("/bingo.html#{fragment}")).into_response();
    response.headers_mut().append(
        header::SET_COOKIE,
        format!(
            "bingo_session={session}; HttpOnly; SameSite=Lax; Path=/; Max-Age={SESSION_SECONDS}{}",
            secure(&app)
        )
        .parse()
        .unwrap(),
    );
    response.headers_mut().append(
        header::SET_COOKIE,
        format!(
            "bingo_oauth=; HttpOnly; SameSite=Lax; Path=/auth/bingo; Max-Age=0{}",
            secure(&app)
        )
        .parse()
        .unwrap(),
    );
    response
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    #[tokio::test]
    async fn viewer_oauth_uses_pkce_browser_binding_and_does_not_register_channels() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        let exchanges = Arc::new(AtomicUsize::new(0));
        let forms = Arc::new(Mutex::new(Vec::<HashMap<String, String>>::new()));
        let (count, captured) = (exchanges.clone(), forms.clone());
        let router = axum::Router::new()
            .route(
                "/token",
                axum::routing::post(
                    move |axum::Form(form): axum::Form<HashMap<String, String>>| {
                        let (count, captured) = (count.clone(), captured.clone());
                        async move {
                            count.fetch_add(1, Ordering::Relaxed);
                            captured.lock().await.push(form);
                            axum::Json(serde_json::json!({"access_token": "test-viewer-token"}))
                        }
                    },
                ),
            )
            .route(
                "/users",
                axum::routing::get(|headers: HeaderMap| async move {
                    assert_eq!(headers[header::AUTHORIZATION], "Bearer test-viewer-token");
                    axum::Json(serde_json::json!({"data": [{"user_id": 123, "name": "SeniorDai"}]}))
                }),
            );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            axum::serve(listener, router).await.unwrap();
        });
        let pool = sqlx::postgres::PgPoolOptions::new()
            .connect_lazy("postgres://test:test@127.0.0.1/unused")
            .unwrap();
        let mut app = crate::test_support::app(pool);
        let app_mut = Arc::get_mut(&mut app).unwrap();
        app_mut.kick_endpoints.oauth = format!("http://{address}");
        app_mut.kick_endpoints.api = format!("http://{address}");
        let token = "a".repeat(32);
        let response = login(
            State(app.clone()),
            Query(LoginQuery {
                ch: "alpha".into(),
                token: token.clone(),
            }),
        )
        .await;
        assert_eq!(response.status(), StatusCode::TEMPORARY_REDIRECT);
        let location =
            url::Url::parse(response.headers()[header::LOCATION].to_str().unwrap()).unwrap();
        let params: HashMap<String, String> = location.query_pairs().into_owned().collect();
        assert_eq!(params["scope"], "user:read");
        assert_eq!(params["code_challenge_method"], "S256");
        assert_eq!(
            params["redirect_uri"],
            "https://test.example/auth/bingo/callback"
        );
        let query = HashMap::from([
            ("state".into(), params["state"].clone()),
            ("code".into(), "test-code".into()),
        ]);
        let wrong_browser =
            callback(State(app.clone()), HeaderMap::new(), Query(query.clone())).await;
        assert_eq!(wrong_browser.status(), StatusCode::BAD_REQUEST);
        assert_eq!(exchanges.load(Ordering::Relaxed), 0);
        let mut headers = HeaderMap::new();
        headers.insert(
            header::COOKIE,
            format!("bingo_oauth={}", params["state"]).parse().unwrap(),
        );
        let response = callback(State(app.clone()), headers.clone(), Query(query.clone())).await;
        assert_eq!(response.status(), StatusCode::SEE_OTHER);
        assert_eq!(
            response.headers()[header::LOCATION],
            format!("/bingo.html#ch=alpha&token={token}")
        );
        let session_cookie = response
            .headers()
            .get_all(header::SET_COOKIE)
            .iter()
            .map(|value| value.to_str().unwrap())
            .find(|value| value.starts_with("bingo_session="))
            .unwrap();
        assert!(session_cookie.contains("HttpOnly; SameSite=Lax; Path=/; Max-Age=28800; Secure"));
        let mut session_headers = HeaderMap::new();
        session_headers.insert(
            header::COOKIE,
            session_cookie.split(';').next().unwrap().parse().unwrap(),
        );
        assert_eq!(
            app.bingo_auth.username(&session_headers).await.as_deref(),
            Some("seniordai")
        );
        let form = &forms.lock().await[0];
        assert_eq!(
            URL_SAFE_NO_PAD.encode(Sha256::digest(form["code_verifier"].as_bytes())),
            params["code_challenge"]
        );
        let replay = callback(State(app.clone()), headers, Query(query)).await;
        assert_eq!(replay.status(), StatusCode::BAD_REQUEST);
        assert_eq!(exchanges.load(Ordering::Relaxed), 1);
        assert!(app.channels.is_empty());
        assert!(app.user_id_to_slug.is_empty());
        server.abort();
    }

    #[tokio::test]
    async fn sessions_require_the_random_cookie_and_expire() {
        let auth = ViewerAuth::default();
        let mut headers = HeaderMap::new();
        assert!(auth.username(&headers).await.is_none());
        let id = auth
            .create_session("viewer".into(), &headers)
            .await
            .unwrap();
        headers.insert(
            header::COOKIE,
            format!("bingo_session={id}").parse().unwrap(),
        );
        assert_eq!(auth.username(&headers).await.as_deref(), Some("viewer"));
        auth.sessions.lock().await.get_mut(&id).unwrap().expires = Instant::now();
        assert!(auth.username(&headers).await.is_none());
    }

    pub(crate) async fn session_headers(auth: &ViewerAuth, user: &str) -> HeaderMap {
        let mut headers = HeaderMap::new();
        let id = auth.create_session(user.into(), &headers).await.unwrap();
        headers.insert(
            header::COOKIE,
            format!("bingo_session={id}").parse().unwrap(),
        );
        headers
    }
}
