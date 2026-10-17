use super::{bad, failure, now, Result, Service};
use axum::{
    extract::{Path, Query, State},
    http::{header, HeaderMap, StatusCode},
    response::{IntoResponse, Redirect, Response},
    Json,
};
use serde::Deserialize;
use serde_json::{json, Value};
use std::{collections::HashMap, sync::Arc};

const OPENID: &str = "https://steamcommunity.com/openid/login";
const NS: &str = "http://specs.openid.net/auth/2.0";
const ID_PREFIX: &str = "https://steamcommunity.com/openid/id/";

pub(super) fn valid_id(id: &str) -> bool {
    id.len() == 17
        && id.bytes().all(|b| b.is_ascii_digit())
        && id.parse::<u64>().is_ok_and(|id| {
            (id >> 56) == 1 && ((id >> 52) & 15) == 1 && ((id >> 32) & 0xfffff) == 1
        })
}

fn cookie<'a>(headers: &'a HeaderMap, name: &str) -> Option<&'a str> {
    headers
        .get(header::COOKIE)?
        .to_str()
        .ok()?
        .split(';')
        .filter_map(|p| p.trim().split_once('='))
        .find_map(|(key, value)| (key == name).then_some(value))
}

fn set_cookie(service: &Service, name: &str, value: &str, age: u32) -> String {
    let secure = if service.app.config.base_url.starts_with("https://") {
        "; Secure"
    } else {
        ""
    };
    format!("{name}={value}; HttpOnly; SameSite=Lax; Path=/; Max-Age={age}{secure}")
}

#[derive(Deserialize)]
pub(super) struct Login {
    ch: String,
}

pub(super) async fn login(
    State(service): State<Arc<Service>>,
    Query(query): Query<Login>,
) -> Result<Response> {
    service.channel(&query.ch)?;
    if service.key.is_empty() {
        return Err((
            StatusCode::SERVICE_UNAVAILABLE,
            "Steam no está configurado: falta STEAM_WEB_API_KEY.".into(),
        ));
    }
    let state = uuid::Uuid::new_v4().simple().to_string();
    let mut tx = service
        .app
        .db
        .begin()
        .await
        .map_err(|e| failure("No se pudo iniciar sesión.", e))?;
    sqlx::query("SELECT pg_advisory_xact_lock(550003)")
        .execute(&mut *tx)
        .await
        .map_err(|e| failure("No se pudo reservar el inicio de sesión.", e))?;
    sqlx::query("DELETE FROM steam_pending WHERE expires < $1")
        .bind(now())
        .execute(&mut *tx)
        .await
        .map_err(|e| failure("No se pudieron limpiar las solicitudes expiradas.", e))?;
    let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM steam_pending")
        .fetch_one(&mut *tx)
        .await
        .map_err(|e| failure("No se pudieron comprobar las solicitudes.", e))?;
    if count >= 10_000 {
        return Err((
            StatusCode::TOO_MANY_REQUESTS,
            "Demasiados inicios de sesión. Intenta más tarde.".into(),
        ));
    }
    sqlx::query("INSERT INTO steam_pending(state,channel,expires) VALUES($1,$2,$3)")
        .bind(&state)
        .bind(&query.ch)
        .bind(now() + 600)
        .execute(&mut *tx)
        .await
        .map_err(|e| failure("No se pudo guardar el inicio de sesión.", e))?;
    tx.commit()
        .await
        .map_err(|e| failure("No se pudo confirmar el inicio de sesión.", e))?;
    let base = service.app.config.base_url.trim_end_matches('/');
    let return_to = format!("{base}/auth/steam/callback?state={state}");
    let params = url::form_urlencoded::Serializer::new(String::new())
        .append_pair("openid.ns", NS)
        .append_pair("openid.mode", "checkid_setup")
        .append_pair("openid.return_to", &return_to)
        .append_pair("openid.realm", &format!("{base}/"))
        .append_pair(
            "openid.identity",
            "http://specs.openid.net/auth/2.0/identifier_select",
        )
        .append_pair(
            "openid.claimed_id",
            "http://specs.openid.net/auth/2.0/identifier_select",
        )
        .finish();
    Ok((
        [(
            header::SET_COOKIE,
            set_cookie(&service, "steam_state", &state, 600),
        )],
        Redirect::temporary(&format!("{OPENID}?{params}")),
    )
        .into_response())
}

fn assertion(params: &HashMap<String, String>, return_to: &str, timestamp: i64) -> Result<String> {
    let get = |key: &str| params.get(key).map(String::as_str).unwrap_or("");
    if get("openid.ns") != NS
        || get("openid.mode") != "id_res"
        || get("openid.op_endpoint") != OPENID
        || get("openid.return_to") != return_to
        || get("openid.identity") != get("openid.claimed_id")
    {
        return Err(bad("Respuesta de identidad Steam inválida."));
    }
    let signed: Vec<_> = get("openid.signed").split(',').collect();
    for required in [
        "op_endpoint",
        "claimed_id",
        "identity",
        "return_to",
        "response_nonce",
        "assoc_handle",
    ] {
        if !signed.contains(&required) {
            return Err(bad("Faltan campos firmados en la respuesta Steam."));
        }
    }
    let nonce = get("openid.response_nonce");
    let time = nonce
        .get(..20)
        .and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok())
        .ok_or_else(|| bad("Nonce de Steam inválido."))?
        .timestamp();
    if !(timestamp - 600..=timestamp + 60).contains(&time) {
        return Err(bad(
            "El inicio de sesión Steam expiró. Vuelve a intentarlo.",
        ));
    }
    let id = get("openid.claimed_id")
        .strip_prefix(ID_PREFIX)
        .filter(|id| valid_id(id))
        .ok_or_else(|| bad("SteamID inválido."))?;
    Ok(id.to_owned())
}

pub(super) async fn callback(
    State(service): State<Arc<Service>>,
    headers: HeaderMap,
    Query(mut params): Query<HashMap<String, String>>,
) -> Result<Response> {
    let state = params
        .remove("state")
        .filter(|s| s.len() == 32 && s.bytes().all(|b| b.is_ascii_hexdigit()))
        .ok_or_else(|| bad("Inicio de sesión Steam inválido."))?;
    if cookie(&headers, "steam_state") != Some(state.as_str()) {
        return Err(bad(
            "La sesión Steam no coincide. Vuelve a iniciar sesión desde este navegador.",
        ));
    }
    let slug = sqlx::query_scalar::<_, String>(
        "DELETE FROM steam_pending WHERE state=$1 AND expires >= $2 RETURNING channel",
    )
    .bind(&state)
    .bind(now())
    .fetch_optional(&service.app.db)
    .await
    .map_err(|e| failure("No se pudo verificar el inicio de sesión.", e))?
    .ok_or_else(|| bad("Inicio de sesión ya usado o expirado."))?;
    service.channel(&slug)?;
    let return_to = format!(
        "{}/auth/steam/callback?state={state}",
        service.app.config.base_url.trim_end_matches('/')
    );
    let steamid = assertion(&params, &return_to, now())?;
    params.insert("openid.mode".into(), "check_authentication".into());
    let verification = service
        .http
        .post(&service.openid_verify)
        .form(&params)
        .send()
        .await
        .map_err(|_| failure("No se pudo verificar la firma con Steam.", "network error"))?;
    if !verification.status().is_success() {
        return Err(failure(
            "Steam rechazó la verificación de identidad.",
            verification.status(),
        ));
    }
    let verification = verification
        .text()
        .await
        .map_err(|_| bad("Respuesta de verificación Steam ilegible."))?;
    if !verification
        .lines()
        .any(|line| line.trim_end_matches('\r') == "is_valid:true")
    {
        return Err((
            StatusCode::UNAUTHORIZED,
            "Steam no confirmó la identidad.".into(),
        ));
    }
    let profile = service.profile(&steamid).await?;
    let mut tx = service
        .app
        .db
        .begin()
        .await
        .map_err(|e| failure("No se pudo guardar tu perfil.", e))?;
    sqlx::query("SELECT pg_advisory_xact_lock(hashtext($1))")
        .bind(format!("steam:{slug}"))
        .execute(&mut *tx)
        .await
        .map_err(|e| failure("No se pudo reservar el perfil.", e))?;
    let count: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM steam_profiles WHERE channel=$1 AND steamid<>$2")
            .bind(&slug)
            .bind(&steamid)
            .fetch_one(&mut *tx)
            .await
            .map_err(|e| failure("No se pudieron comprobar los perfiles.", e))?;
    if count >= 32 {
        return Err(bad(
            "Este canal ya tiene 32 perfiles. Desvincula uno antes de añadir otro.",
        ));
    }
    sqlx::query("INSERT INTO steam_profiles(channel,steamid,profile,updated) VALUES($1,$2,$3,$4) ON CONFLICT(channel,steamid) DO UPDATE SET profile=$3,updated=$4")
        .bind(&slug).bind(&steamid).bind(profile.to_string()).bind(now()).execute(&mut *tx).await
        .map_err(|e| failure("No se pudo guardar el perfil Steam.", e))?;
    if let Some(old) = cookie(&headers, "steam_session") {
        sqlx::query("DELETE FROM steam_sessions WHERE id=$1")
            .bind(old)
            .execute(&mut *tx)
            .await
            .map_err(|e| failure("No se pudo renovar la sesión Steam.", e))?;
    }
    // Keep one active web session per profile and channel.
    sqlx::query("DELETE FROM steam_sessions WHERE channel=$1 AND steamid=$2")
        .bind(&slug)
        .bind(&steamid)
        .execute(&mut *tx)
        .await
        .map_err(|e| failure("No se pudo renovar tu sesión.", e))?;
    let session = uuid::Uuid::new_v4().simple().to_string();
    sqlx::query("INSERT INTO steam_sessions(id,channel,steamid,expires) VALUES($1,$2,$3,$4)")
        .bind(&session)
        .bind(&slug)
        .bind(&steamid)
        .bind(now() + 28800)
        .execute(&mut *tx)
        .await
        .map_err(|e| failure("No se pudo crear tu sesión Steam.", e))?;
    tx.commit()
        .await
        .map_err(|e| failure("No se pudo confirmar tu perfil Steam.", e))?;
    let query = url::form_urlencoded::Serializer::new(String::new())
        .append_pair("ch", &slug)
        .finish();
    let mut response = Redirect::to(&format!("/l4d2.html?{query}")).into_response();
    response.headers_mut().append(
        header::SET_COOKIE,
        set_cookie(&service, "steam_session", &session, 28800)
            .parse()
            .unwrap(),
    );
    response.headers_mut().append(
        header::SET_COOKIE,
        set_cookie(&service, "steam_state", "", 0).parse().unwrap(),
    );
    Ok(response)
}

impl Service {
    async fn api(
        &self,
        method: &str,
        params: &[(&str, String)],
    ) -> std::result::Result<Value, String> {
        let response = self
            .http
            .get(format!("{}/{method}", self.steam_api))
            .query(&[("key", &self.key)])
            .query(params)
            .send()
            .await
            .map_err(|_| "No se pudo conectar con Steam.".to_string())?;
        if !response.status().is_success() {
            return Err(format!("Steam respondió {}. Revisa la clave API y la privacidad del perfil/detalles de juego.", response.status()));
        }
        response
            .json()
            .await
            .map_err(|_| "Steam devolvió datos ilegibles.".to_string())
    }

    pub(super) async fn profile(&self, steamid: &str) -> Result<Value> {
        let summary = self
            .api(
                "ISteamUser/GetPlayerSummaries/v2/",
                &[("steamids", steamid.to_owned())],
            )
            .await
            .map_err(|e| failure("No se pudo consultar el perfil Steam.", e))?;
        let user = summary["response"]["players"]
            .as_array()
            .and_then(|players| players.iter().find(|p| p["steamid"] == steamid))
            .ok_or_else(|| bad("Steam no devolvió el perfil solicitado."))?;
        let mut result = json!({
            "steamid":steamid, "name":user["personaname"], "avatar":user["avatarfull"],
            "profile_url":format!("https://steamcommunity.com/profiles/{steamid}/"),
            "stats":[], "stats_error":null, "achievements":null, "achievements_error":null,
            "minutes":null, "playtime_error":null
        });
        let stats = self
            .api(
                "ISteamUserStats/GetUserStatsForGame/v2/",
                &[("steamid", steamid.to_owned()), ("appid", "550".into())],
            )
            .await;
        match stats {
            Ok(data)
                if data["playerstats"]["steamID"] == steamid
                    && data["playerstats"]["stats"].is_array() =>
            {
                result["stats"] = Value::Array(
                    data["playerstats"]["stats"]
                        .as_array()
                        .unwrap()
                        .iter()
                        .filter(|stat| stat["name"].is_string() && stat["value"].is_number())
                        .take(256)
                        .cloned()
                        .collect(),
                );
            }
            Ok(_) => {
                result["stats_error"] = json!("Steam no expone estadísticas L4D2 para este perfil.")
            }
            Err(error) => result["stats_error"] = json!(error),
        }
        match self
            .api(
                "ISteamUserStats/GetPlayerAchievements/v1/",
                &[
                    ("steamid", steamid.to_owned()),
                    ("appid", "550".into()),
                    ("l", "spanish".into()),
                ],
            )
            .await
        {
            Ok(data) => {
                if let Some(items) = data["playerstats"]["achievements"].as_array() {
                    result["achievements"] = json!({"unlocked":items.iter().filter(|a| a["achieved"] == 1).count(),"total":items.len()});
                } else {
                    result["achievements_error"] = json!("Logros L4D2 no disponibles.");
                }
            }
            Err(error) => result["achievements_error"] = json!(error),
        }
        let input =
            json!({"steamid":steamid,"appids_filter":[550],"include_played_free_games":true})
                .to_string();
        match self
            .api("IPlayerService/GetOwnedGames/v1/", &[("input_json", input)])
            .await
        {
            Ok(data) => {
                if let Some(game) = data["response"]["games"]
                    .as_array()
                    .and_then(|games| games.iter().find(|g| g["appid"] == 550))
                {
                    if let Some(minutes) = game["playtime_forever"].as_u64() {
                        result["minutes"] = json!(minutes);
                    } else {
                        result["playtime_error"] = json!("Horas de juego no disponibles.");
                    }
                } else {
                    result["playtime_error"] = json!("Biblioteca privada o L4D2 no disponible.");
                }
            }
            Err(error) => result["playtime_error"] = json!(error),
        }
        Ok(result)
    }
}

async fn identity(service: &Service, slug: &str, headers: &HeaderMap) -> Result<Option<String>> {
    service.channel(slug)?;
    let Some(session) = cookie(headers, "steam_session") else {
        return Ok(None);
    };
    sqlx::query_scalar(
        "SELECT steamid FROM steam_sessions WHERE id=$1 AND channel=$2 AND expires >= $3",
    )
    .bind(session)
    .bind(slug)
    .bind(now())
    .fetch_optional(&service.app.db)
    .await
    .map_err(|e| failure("No se pudo consultar la sesión Steam.", e))
}

pub(super) async fn account(
    State(service): State<Arc<Service>>,
    Path(slug): Path<String>,
    headers: HeaderMap,
) -> Result<Json<Value>> {
    Ok(Json(
        json!({"steamid":identity(&service, &slug, &headers).await?}),
    ))
}

pub(super) async fn unlink(
    State(service): State<Arc<Service>>,
    Path(slug): Path<String>,
    headers: HeaderMap,
) -> Result<Response> {
    service.same_origin(&headers)?;
    let steamid = identity(&service, &slug, &headers).await?.ok_or((
        StatusCode::UNAUTHORIZED,
        "Inicia sesión con Steam para retirar tu perfil.".into(),
    ))?;
    sqlx::query("DELETE FROM steam_profiles WHERE channel=$1 AND steamid=$2")
        .bind(slug)
        .bind(steamid)
        .execute(&service.app.db)
        .await
        .map_err(|e| failure("No se pudo retirar tu perfil Steam.", e))?;
    Ok((
        [(
            header::SET_COOKIE,
            set_cookie(&service, "steam_session", "", 0),
        )],
        StatusCode::NO_CONTENT,
    )
        .into_response())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::l4d2::{settings, Change};

    fn service(pool: sqlx::PgPool, base: &str) -> Arc<Service> {
        let app = crate::test_support::app(pool);
        let (channel, _) = crate::test_support::channel("alpha", 1);
        app.channels.insert("alpha".into(), channel);
        Arc::new(Service {
            app,
            http: reqwest::Client::builder()
                .redirect(reqwest::redirect::Policy::none())
                .build()
                .unwrap(),
            key: "test-key-not-a-secret".into(),
            servers: vec!["8.8.8.8:27015".parse().unwrap()],
            steam_api: base.into(),
            openid_verify: format!("{base}/verify"),
        })
    }

    async fn mock_steam(private: bool) -> crate::test_support::HttpServer {
        use axum::routing::{get, post};
        crate::test_support::HttpServer::start(axum::Router::new()
            .route("/verify", post(|axum::Form(params): axum::Form<HashMap<String,String>>| async move {
                assert_eq!(params["openid.mode"], "check_authentication");
                "is_valid:true\n"
            }))
            .route("/ISteamUser/GetPlayerSummaries/v2/", get(|| async {
                Json(json!({"response":{"players":[{"steamid":"76561197960265729","personaname":"<script>Alice</script>","avatarfull":"https://avatars.steamstatic.com/test.jpg"}]}}))
            }))
            .route("/ISteamUserStats/GetUserStatsForGame/v2/", get(move || async move {
                if private { return (StatusCode::FORBIDDEN, Json(json!({}))); }
                (StatusCode::OK, Json(json!({"playerstats":{"steamID":"76561197960265729","stats":[{"name":"kills","value":123}]}})))
            }))
            .route("/ISteamUserStats/GetPlayerAchievements/v1/", get(|| async {
                Json(json!({"playerstats":{"achievements":[{"achieved":1},{"achieved":0}]}}))
            }))
            .route("/IPlayerService/GetOwnedGames/v1/", get(|| async {
                Json(json!({"response":{"games":[{"appid":550,"playtime_forever":120}]}}))
            }))).await
    }

    #[tokio::test]
    async fn steam_api_keeps_private_stats_unavailable_instead_of_zero() {
        let mock = mock_steam(true).await;
        let pool = sqlx::postgres::PgPoolOptions::new()
            .connect_lazy("postgres://localhost/unused")
            .unwrap();
        let profile = service(pool, &mock.url)
            .profile("76561197960265729")
            .await
            .unwrap();
        assert_eq!(profile["name"], "<script>Alice</script>");
        assert_eq!(profile["stats"], json!([]));
        assert!(profile["stats_error"].as_str().unwrap().contains("403"));
        assert_eq!(profile["minutes"], 120);
        assert_eq!(profile["achievements"], json!({"unlocked":1,"total":2}));
    }

    #[tokio::test]
    async fn panel_requires_correct_scope_and_origin_before_any_database_access() {
        let pool = sqlx::postgres::PgPoolOptions::new()
            .connect_lazy("postgres://localhost/unused")
            .unwrap();
        let service = service(pool, "http://localhost");
        let mut headers = HeaderMap::new();
        headers.insert(header::AUTHORIZATION, "Bearer play-secret".parse().unwrap());
        let change = || {
            Json(Change::Server {
                address: "8.8.8.8:27015".into(),
            })
        };
        let result = settings(
            State(service.clone()),
            Path("alpha".into()),
            headers.clone(),
            change(),
        )
        .await
        .unwrap_err();
        assert_eq!(result.0, StatusCode::UNAUTHORIZED);
        headers.insert(
            header::AUTHORIZATION,
            "Bearer panel-secret".parse().unwrap(),
        );
        let result = settings(
            State(service.clone()),
            Path("alpha".into()),
            headers.clone(),
            change(),
        )
        .await
        .unwrap_err();
        assert_eq!(result.0, StatusCode::FORBIDDEN);
        headers.insert(header::ORIGIN, "https://test.example".parse().unwrap());
        let result = settings(
            State(service.clone()),
            Path("alpha".into()),
            headers,
            Json(Change::Server {
                address: "127.0.0.1:27015".into(),
            }),
        )
        .await
        .unwrap_err();
        assert_eq!(result.0, StatusCode::BAD_REQUEST);
        let result = callback(
            State(service),
            HeaderMap::new(),
            Query(HashMap::from([("state".into(), "a".repeat(32))])),
        )
        .await
        .unwrap_err();
        assert_eq!(result.0, StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    #[ignore = "requires TEST_DATABASE_URL pointing to an isolated PostgreSQL database"]
    async fn login_persists_profile_links_and_session_and_unlink_removes_personal_data() {
        let database = crate::test_support::TestDatabase::new().await;
        crate::db::upsert_channel(&database.pool, &crate::test_support::row("alpha", 1))
            .await
            .unwrap();
        let mock = mock_steam(false).await;
        let service = service(database.pool.clone(), &mock.url);
        let server = crate::test_support::HttpServer::start(
            crate::l4d2::routes(service.clone()).with_state(service.app.clone()),
        )
        .await;
        let client = reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .unwrap();
        let response = client
            .get(format!("{}/auth/steam?ch=alpha", server.url))
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::TEMPORARY_REDIRECT);
        let location =
            url::Url::parse(response.headers()[header::LOCATION].to_str().unwrap()).unwrap();
        let query: HashMap<String, String> = location.query_pairs().into_owned().collect();
        let return_to = url::Url::parse(&query["openid.return_to"]).unwrap();
        let state = return_to
            .query_pairs()
            .find(|(k, _)| k == "state")
            .unwrap()
            .1
            .into_owned();
        let mut assertion = params();
        assertion.insert("state".into(), state.clone());
        assertion.insert("openid.return_to".into(), query["openid.return_to"].clone());
        assertion.insert(
            "openid.response_nonce".into(),
            format!("{}nonce", chrono::Utc::now().format("%Y-%m-%dT%H:%M:%SZ")),
        );
        let url = format!("{}/auth/steam/callback", server.url);
        assert_eq!(
            client
                .get(&url)
                .query(&assertion)
                .send()
                .await
                .unwrap()
                .status(),
            StatusCode::BAD_REQUEST
        );
        let response = client
            .get(&url)
            .header(header::COOKIE, format!("steam_state={state}"))
            .query(&assertion)
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::SEE_OTHER);
        let session = response
            .headers()
            .get_all(header::SET_COOKIE)
            .iter()
            .map(|v| v.to_str().unwrap())
            .find(|v| v.starts_with("steam_session="))
            .unwrap()
            .split(';')
            .next()
            .unwrap()
            .to_string();
        assert_eq!(
            client
                .get(&url)
                .header(header::COOKIE, format!("steam_state={state}"))
                .query(&assertion)
                .send()
                .await
                .unwrap()
                .status(),
            StatusCode::BAD_REQUEST
        );
        let api = format!("{}/api/l4d2/alpha", server.url);
        let snapshot: Value = client.get(&api).send().await.unwrap().json().await.unwrap();
        assert_eq!(snapshot["profiles"][0]["profile"]["stats"][0]["value"], 123);
        let user: Value = client
            .get(format!("{api}/account"))
            .header(header::COOKIE, &session)
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
        assert_eq!(user["steamid"], "76561197960265729");
        sqlx::query(
            "INSERT INTO l4d2_settings(channel,server,snapshot) VALUES('alpha','8.8.8.8:27015',$1)",
        )
        .bind(json!({"updated":now(),"server":{"players":[{"name":"Alice"}]}}).to_string())
        .execute(&database.pool)
        .await
        .unwrap();
        let response = client
            .post(&api)
            .header(header::AUTHORIZATION, "Bearer panel-secret")
            .header(header::ORIGIN, "https://test.example")
            .json(&json!({"action":"link","name":"Alice","steamid":"76561197960265729"}))
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::NO_CONTENT);
        let snapshot: Value = client.get(&api).send().await.unwrap().json().await.unwrap();
        assert_eq!(snapshot["links"][0]["verified"], false);
        assert!(!snapshot.to_string().contains("test-key-not-a-secret"));
        assert!(!snapshot.to_string().contains("panel-secret"));
        let duplicate =
            json!({"updated":now(),"server":{"players":[{"name":"Alice"},{"name":"Alice"}]}});
        sqlx::query("UPDATE l4d2_settings SET snapshot=$1 WHERE channel='alpha'")
            .bind(duplicate.to_string())
            .execute(&database.pool)
            .await
            .unwrap();
        let response = client
            .post(&api)
            .header(header::AUTHORIZATION, "Bearer panel-secret")
            .header(header::ORIGIN, "https://test.example")
            .json(&json!({"action":"link","name":"Alice","steamid":"76561197960265729"}))
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        let response = client
            .post(&api)
            .header(header::AUTHORIZATION, "Bearer panel-secret")
            .header(header::ORIGIN, "https://test.example")
            .json(&json!({"action":"server","address":""}))
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::NO_CONTENT);
        let snapshot: Value = client.get(&api).send().await.unwrap().json().await.unwrap();
        assert_eq!(snapshot["links"], json!([]));
        // Recreate an association to verify profile removal cascades as well.
        sqlx::query("INSERT INTO l4d2_links(channel,player_name,steamid) VALUES('alpha','Alice','76561197960265729')")
            .execute(&database.pool).await.unwrap();
        let response = client
            .delete(format!("{api}/account"))
            .header(header::COOKIE, &session)
            .header(header::ORIGIN, "https://test.example")
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::NO_CONTENT);
        let snapshot: Value = client.get(&api).send().await.unwrap().json().await.unwrap();
        assert_eq!(snapshot["profiles"], json!([]));
        assert_eq!(snapshot["links"], json!([]));
        let user: Value = client
            .get(format!("{api}/account"))
            .header(header::COOKIE, &session)
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
        assert!(user["steamid"].is_null());
        drop(server);
        drop(service);
        database.cleanup().await;
    }
    fn params() -> HashMap<String, String> {
        [
            ("openid.ns", NS),
            ("openid.mode", "id_res"),
            ("openid.op_endpoint", OPENID),
            (
                "openid.return_to",
                "https://bot.example/auth/steam/callback?state=abc",
            ),
            (
                "openid.identity",
                "https://steamcommunity.com/openid/id/76561197960265729",
            ),
            (
                "openid.claimed_id",
                "https://steamcommunity.com/openid/id/76561197960265729",
            ),
            (
                "openid.signed",
                "op_endpoint,claimed_id,identity,return_to,response_nonce,assoc_handle",
            ),
            ("openid.response_nonce", "2026-10-17T05:00:00Znonce"),
        ]
        .into_iter()
        .map(|(k, v)| (k.into(), v.into()))
        .collect()
    }
    #[test]
    fn validates_signed_fields_endpoint_return_url_id_and_nonce() {
        let data = params();
        let time = chrono::DateTime::parse_from_rfc3339("2026-10-17T05:00:00Z")
            .unwrap()
            .timestamp();
        let url = &data["openid.return_to"];
        assert_eq!(assertion(&data, url, time).unwrap(), "76561197960265729");
        assert!(assertion(&data, url, time + 601).is_err());
        assert!(assertion(&data, "https://evil.example", time).is_err());
        for field in [
            "openid.ns",
            "openid.mode",
            "openid.op_endpoint",
            "openid.signed",
            "openid.claimed_id",
            "openid.identity",
            "openid.response_nonce",
        ] {
            let mut invalid = data.clone();
            invalid.insert(field.into(), "invalid".into());
            assert!(assertion(&invalid, url, time).is_err(), "{field}");
        }
        assert!(!valid_id("12345678901234567"));
        assert!(!valid_id("<script>"));
    }
}
