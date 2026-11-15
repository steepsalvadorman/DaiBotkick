//! API de los paneles (docks) de OBS: chat y configuración del stream.
//!
//! Todas las rutas exigen `Authorization: Bearer <panel_token>` del canal. El token viaja en
//! el fragmento de la URL del dock y nunca en la query, para que no quede en logs ni referers.
use crate::{
    platform::Platform,
    state::{AppState, ChannelState},
};
use axum::{
    extract::{Path, Query, State},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    Json,
};
use std::sync::Arc;

const TITLE_MAX: usize = 140;
const CHAT_MAX: usize = 500;

type Reply = Result<Response, (StatusCode, &'static str)>;

fn channel(
    app: &AppState,
    slug: &str,
    headers: &HeaderMap,
) -> Result<Arc<ChannelState>, (StatusCode, &'static str)> {
    check(app.channels.get(slug).map(|c| c.clone()), headers)
}

fn check(
    ch: Option<Arc<ChannelState>>,
    headers: &HeaderMap,
) -> Result<Arc<ChannelState>, (StatusCode, &'static str)> {
    let supplied = headers
        .get(axum::http::header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "));
    match ch.filter(|c| !c.cancel.is_cancelled()) {
        Some(ch) if crate::server::authorized(&ch.panel_token, supplied) => Ok(ch),
        _ => Err((
            StatusCode::UNAUTHORIZED,
            "Panel no autorizado. Copia de nuevo la URL del dock.",
        )),
    }
}

/// Petición a la API de Kick con el token del canal; renueva una vez si Kick responde 401.
async fn kick(
    ch: &Arc<ChannelState>,
    app: &Arc<AppState>,
    build: impl Fn(&reqwest::Client, &str) -> reqwest::RequestBuilder,
) -> Result<reqwest::Response, (StatusCode, &'static str)> {
    for attempt in 0..2 {
        let token = ch.access_token.read().await.clone();
        let response = build(&app.http, &app.kick_endpoints.api)
            .bearer_auth(&token)
            .send()
            .await
            .map_err(|_| (StatusCode::BAD_GATEWAY, "Kick no responde"))?;
        if response.status() == StatusCode::UNAUTHORIZED
            && attempt == 0
            && crate::kick::refresh_if_current(ch, app, &token).await
        {
            continue;
        }
        return match response.status() {
            s if s.is_success() => Ok(response),
            StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN => Err((
                StatusCode::FORBIDDEN,
                "Kick rechazó el permiso. Vuelve a conectar el canal para autorizar los paneles.",
            )),
            s => {
                tracing::warn!("[Dock][{}] Kick HTTP {s}", ch.slug);
                Err((StatusCode::BAD_GATEWAY, "Kick rechazó la solicitud"))
            }
        };
    }
    Err((StatusCode::BAD_GATEWAY, "Kick rechazó la solicitud"))
}

/// Petición a Helix (Twitch) con el token del canal; renueva una vez si Twitch responde 401.
async fn helix(
    ch: &Arc<ChannelState>,
    app: &Arc<AppState>,
    build: impl Fn(&reqwest::Client, &str) -> reqwest::RequestBuilder,
) -> Result<serde_json::Value, (StatusCode, &'static str)> {
    let response = crate::twitch::helix(ch, app, build)
        .await
        .map_err(|e| (e.status, e.message))?;
    if response.status() == StatusCode::NO_CONTENT {
        return Ok(serde_json::Value::Null);
    }
    response
        .json()
        .await
        .map_err(|_| (StatusCode::BAD_GATEWAY, "Respuesta de Twitch inválida"))
}

/// Carátula de una categoría de Twitch en el tamaño de las de Kick (52×72).
fn box_art(url: &str) -> String {
    url.replace("{width}x{height}", "52x72")
}

/// Título, categoría y estado del directo en Twitch, con la misma forma que `stream_summary`.
async fn twitch_summary(
    ch: &Arc<ChannelState>,
    app: &Arc<AppState>,
    id: u64,
) -> Result<serde_json::Value, (StatusCode, &'static str)> {
    let id = id.to_string();
    let channel = helix(ch, app, |http, api| {
        http.get(format!("{api}/channels"))
            .query(&[("broadcaster_id", &id)])
    })
    .await?;
    let stream = helix(ch, app, |http, api| {
        http.get(format!("{api}/streams"))
            .query(&[("user_id", &id)])
    })
    .await?;
    let channel = &channel["data"][0];
    let game_id = channel["game_id"]
        .as_str()
        .and_then(|g| g.parse::<u64>().ok());
    let thumbnail = match game_id {
        Some(game) => helix(ch, app, |http, api| {
            http.get(format!("{api}/games"))
                .query(&[("id", game.to_string())])
        })
        .await
        .ok()
        .and_then(|g| g["data"][0]["box_art_url"].as_str().map(box_art))
        .unwrap_or_default(),
        None => String::new(),
    };
    let stream = &stream["data"][0];
    let live = stream["type"] == "live";
    Ok(serde_json::json!({
        "title": channel["title"].as_str().unwrap_or(""),
        "category": game_id.map(|id| serde_json::json!({
            "id": id,
            "name": channel["game_name"].as_str().unwrap_or(""),
            "thumbnail": thumbnail,
        })),
        "live": live,
        "viewers": stream["viewer_count"].as_u64().filter(|_| live),
    }))
}

/// Cuerpo de `PATCH /helix/channels`: Twitch usa `title` y `game_id` (texto).
fn twitch_update_body(kick: &serde_json::Value) -> serde_json::Value {
    let mut body = serde_json::Map::new();
    if let Some(title) = kick["stream_title"].as_str() {
        body.insert("title".into(), title.into());
    }
    if let Some(id) = kick["category_id"].as_u64() {
        body.insert("game_id".into(), id.to_string().into());
    }
    body.into()
}

fn finish(result: Reply) -> Response {
    result.unwrap_or_else(|(status, msg)| {
        (status, Json(serde_json::json!({"error": msg}))).into_response()
    })
}

/// Título, categoría y estado actual del directo.
pub async fn stream_info(
    State(app): State<Arc<AppState>>,
    Path(slug): Path<String>,
    headers: HeaderMap,
) -> Response {
    finish(
        async {
            let ch = channel(&app, &slug, &headers)?;
            let Some(id) = *ch.channel_id.read().await else {
                return Err((
                    StatusCode::SERVICE_UNAVAILABLE,
                    "Canal aún sin identificador",
                ));
            };
            if ch.platform == Platform::Twitch {
                return Ok(Json(twitch_summary(&ch, &app, id).await?).into_response());
            }
            let json: serde_json::Value = kick(&ch, &app, |http, api| {
                http.get(format!("{api}/channels?broadcaster_user_id={id}"))
            })
            .await?
            .json()
            .await
            .map_err(|_| (StatusCode::BAD_GATEWAY, "Respuesta de Kick inválida"))?;
            Ok(Json(stream_summary(&json["data"][0])).into_response())
        }
        .await,
    )
}

fn stream_summary(c: &serde_json::Value) -> serde_json::Value {
    serde_json::json!({
        "title": c["stream_title"].as_str().unwrap_or(""),
        "category": c["category"]["id"].as_u64().map(|id| serde_json::json!({
            "id": id,
            "name": c["category"]["name"].as_str().unwrap_or(""),
            "thumbnail": c["category"]["thumbnail"].as_str().unwrap_or(""),
        })),
        "live": c["stream"]["is_live"].as_bool().unwrap_or(false),
        "viewers": c["stream"]["viewer_count"].as_u64(),
    })
}

#[derive(serde::Deserialize)]
pub struct StreamUpdate {
    title: Option<String>,
    category_id: Option<u64>,
}

/// Valida y arma el cuerpo de `PATCH /channels`.
fn update_body(update: &StreamUpdate) -> Result<serde_json::Value, (StatusCode, &'static str)> {
    let mut body = serde_json::Map::new();
    if let Some(title) = update.title.as_deref().map(str::trim) {
        if title.is_empty() || title.chars().count() > TITLE_MAX {
            return Err((
                StatusCode::BAD_REQUEST,
                "El título debe tener entre 1 y 140 caracteres",
            ));
        }
        body.insert("stream_title".into(), title.into());
    }
    if let Some(id) = update.category_id {
        if id == 0 {
            return Err((StatusCode::BAD_REQUEST, "Categoría inválida"));
        }
        body.insert("category_id".into(), id.into());
    }
    if body.is_empty() {
        return Err((StatusCode::BAD_REQUEST, "No hay cambios"));
    }
    Ok(body.into())
}

pub async fn update_stream(
    State(app): State<Arc<AppState>>,
    Path(slug): Path<String>,
    headers: HeaderMap,
    Json(update): Json<StreamUpdate>,
) -> Response {
    finish(
        async {
            let ch = channel(&app, &slug, &headers)?;
            let body = update_body(&update)?;
            match ch.platform {
                Platform::Kick => {
                    kick(&ch, &app, |http, api| {
                        http.patch(format!("{api}/channels")).json(&body)
                    })
                    .await?;
                }
                Platform::Twitch => {
                    let id = ch.channel_id.read().await.unwrap_or(0).to_string();
                    let body = twitch_update_body(&body);
                    helix(&ch, &app, |http, api| {
                        http.patch(format!("{api}/channels"))
                            .query(&[("broadcaster_id", &id)])
                            .json(&body)
                    })
                    .await?;
                }
            }
            tracing::info!("[Dock][{}] Stream actualizado", ch.slug);
            Ok(StatusCode::NO_CONTENT.into_response())
        }
        .await,
    )
}

#[derive(serde::Deserialize)]
pub struct CategoryQuery {
    q: String,
}

pub async fn categories(
    State(app): State<Arc<AppState>>,
    Path(slug): Path<String>,
    headers: HeaderMap,
    Query(query): Query<CategoryQuery>,
) -> Response {
    finish(
        async {
            let ch = channel(&app, &slug, &headers)?;
            let q: String = query.q.trim().chars().take(100).collect();
            if q.chars().count() < 2 {
                return Ok(Json(serde_json::json!({"data": []})).into_response());
            }
            if ch.platform == Platform::Twitch {
                let json = helix(&ch, &app, |http, api| {
                    http.get(format!("{api}/search/categories"))
                        .query(&[("query", q.as_str()), ("first", "20")])
                })
                .await?;
                return Ok(Json(twitch_categories(&json)).into_response());
            }
            let json: serde_json::Value = kick(&ch, &app, |http, api| {
                http.get(format!("{api}/categories"))
                    .query(&[("q", q.as_str())])
            })
            .await?
            .json()
            .await
            .map_err(|_| (StatusCode::BAD_GATEWAY, "Respuesta de Kick inválida"))?;
            let data: Vec<_> = json["data"]
                .as_array()
                .map(|a| {
                    a.iter()
                        .take(20)
                        .filter_map(|c| {
                            Some(serde_json::json!({
                                "id": c["id"].as_u64()?,
                                "name": c["name"].as_str()?,
                                "thumbnail": c["thumbnail"].as_str().unwrap_or(""),
                            }))
                        })
                        .collect()
                })
                .unwrap_or_default();
            Ok(Json(serde_json::json!({"data": data})).into_response())
        }
        .await,
    )
}

/// Resultados de `GET /helix/search/categories` con la forma que espera el dock.
fn twitch_categories(json: &serde_json::Value) -> serde_json::Value {
    let data: Vec<_> = json["data"]
        .as_array()
        .into_iter()
        .flatten()
        .take(20)
        .filter_map(|c| {
            Some(serde_json::json!({
                "id": c["id"].as_str()?.parse::<u64>().ok()?,
                "name": c["name"].as_str()?,
                "thumbnail": c["box_art_url"].as_str().map(box_art).unwrap_or_default(),
            }))
        })
        .collect();
    serde_json::json!({ "data": data })
}

#[derive(serde::Deserialize)]
pub struct ChatSend {
    content: String,
}

/// Escribe en el chat como el propio streamer (no como DaiBot).
pub async fn send_chat(
    State(app): State<Arc<AppState>>,
    Path(slug): Path<String>,
    headers: HeaderMap,
    Json(msg): Json<ChatSend>,
) -> Response {
    finish(
        async {
            let ch = channel(&app, &slug, &headers)?;
            let content = msg.content.trim();
            if content.is_empty() || content.chars().count() > CHAT_MAX {
                return Err((
                    StatusCode::BAD_REQUEST,
                    "El mensaje debe tener entre 1 y 500 caracteres",
                ));
            }
            // En Twitch el dock escribe como DaiBotsito: el canal no autoriza escribir como él.
            if ch.platform == Platform::Twitch {
                crate::twitch::sender::send(content, &ch, &app).await;
                return Ok(StatusCode::NO_CONTENT.into_response());
            }
            let Some(id) = *ch.channel_id.read().await else {
                return Err((
                    StatusCode::SERVICE_UNAVAILABLE,
                    "Canal aún sin identificador",
                ));
            };
            let body =
                serde_json::json!({"broadcaster_user_id": id, "content": content, "type": "user"});
            kick(&ch, &app, |http, api| {
                http.post(format!("{api}/chat")).json(&body)
            })
            .await?;
            Ok(StatusCode::NO_CONTENT.into_response())
        }
        .await,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn upd(title: Option<&str>, category_id: Option<u64>) -> StreamUpdate {
        StreamUpdate {
            title: title.map(Into::into),
            category_id,
        }
    }

    #[test]
    fn stream_updates_are_validated() {
        assert!(update_body(&upd(None, None)).is_err());
        assert!(update_body(&upd(Some("   "), None)).is_err());
        assert!(update_body(&upd(Some(&"a".repeat(141)), None)).is_err());
        assert!(update_body(&upd(None, Some(0))).is_err());
        let body = update_body(&upd(Some("  Jugando  "), Some(15))).unwrap();
        assert_eq!(
            body,
            serde_json::json!({"stream_title":"Jugando","category_id":15})
        );
    }

    #[test]
    fn summary_reads_kick_channel_shape() {
        let c = serde_json::json!({
            "stream_title": "Hola",
            "category": {"id": 3, "name": "Just Chatting", "thumbnail": "https://x/y.jpg"},
            "stream": {"is_live": true, "viewer_count": 12}
        });
        let s = stream_summary(&c);
        assert_eq!(s["title"], "Hola");
        assert_eq!(s["category"]["name"], "Just Chatting");
        assert_eq!(s["live"], true);
        assert_eq!(s["viewers"], 12);
        assert_eq!(
            stream_summary(&serde_json::Value::Null)["category"],
            serde_json::Value::Null
        );
    }

    #[test]
    fn twitch_categories_and_updates_use_twitch_field_names() {
        let json = serde_json::json!({"data": [
            {"id": "509658", "name": "Just Chatting", "box_art_url": "https://static-cdn.jtvnw.net/ttv-boxart/509658-{width}x{height}.jpg"},
            {"id": "x", "name": "sin id numérico"},
            {"id": "21779", "name": "League of Legends"}
        ]});
        let data = twitch_categories(&json);
        assert_eq!(data["data"].as_array().unwrap().len(), 2);
        assert_eq!(data["data"][0]["id"], 509658);
        assert_eq!(
            data["data"][0]["thumbnail"],
            "https://static-cdn.jtvnw.net/ttv-boxart/509658-52x72.jpg"
        );
        let kick = update_body(&upd(Some(" Jugando "), Some(509658))).unwrap();
        assert_eq!(
            twitch_update_body(&kick),
            serde_json::json!({"title": "Jugando", "game_id": "509658"})
        );
        assert_eq!(
            twitch_update_body(&update_body(&upd(None, Some(5))).unwrap()),
            serde_json::json!({"game_id": "5"})
        );
    }

    #[test]
    fn dock_requires_the_panel_token() {
        let (ch, _rx) = crate::test_support::channel("alpha", 1);
        let mut headers = HeaderMap::new();
        assert!(check(Some(ch.clone()), &headers).is_err());
        headers.insert("authorization", "Bearer wrong".parse().unwrap());
        assert!(check(Some(ch.clone()), &headers).is_err());
        headers.insert("authorization", "Bearer panel-secret".parse().unwrap());
        assert!(check(None, &headers).is_err());
        assert!(check(Some(ch.clone()), &headers).is_ok());
        ch.cancel.cancel();
        assert!(check(Some(ch), &headers).is_err());
    }
}
