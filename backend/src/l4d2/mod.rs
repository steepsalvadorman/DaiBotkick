mod a2s;
mod steam;

use crate::state::AppState;
use axum::{
    extract::{Path, State},
    http::{header, HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    routing::get,
    Json, Router,
};
use serde::Deserialize;
use serde_json::{json, Value};
use std::{net::SocketAddr, sync::Arc, time::Duration};

pub(super) struct Service {
    app: Arc<AppState>,
    http: reqwest::Client,
    key: String,
    servers: Vec<SocketAddr>,
    steam_api: String,
    openid_verify: String,
}

type Error = (StatusCode, String);
type Result<T> = std::result::Result<T, Error>;

fn failure(message: &str, error: impl std::fmt::Display) -> Error {
    tracing::error!("[L4D2] {message}: {error}");
    (StatusCode::SERVICE_UNAVAILABLE, message.to_owned())
}

fn bad(message: &str) -> Error {
    (StatusCode::BAD_REQUEST, message.to_owned())
}

fn now() -> i64 {
    chrono::Utc::now().timestamp()
}

pub fn router(app: Arc<AppState>) -> std::result::Result<Router<Arc<AppState>>, String> {
    let servers = std::env::var("STEAM_A2S_SERVERS")
        .unwrap_or_default()
        .split(',')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(|s| {
            s.parse::<SocketAddr>()
                .map_err(|_| format!("STEAM_A2S_SERVERS: dirección inválida: {s}"))
        })
        .collect::<std::result::Result<Vec<_>, _>>()?;
    if servers.len() > 16
        || servers
            .iter()
            .any(|address| address.port() == 0 || !public_address(address.ip()))
    {
        return Err(
            "STEAM_A2S_SERVERS admite hasta 16 IP:puerto públicas, sin DNS ni redes privadas."
                .into(),
        );
    }
    let service = Arc::new(Service {
        app,
        http: reqwest::Client::builder()
            .timeout(Duration::from_secs(12))
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .map_err(|e| e.to_string())?,
        key: std::env::var("STEAM_WEB_API_KEY").unwrap_or_default(),
        servers,
        steam_api: "https://api.steampowered.com".into(),
        openid_verify: "https://steamcommunity.com/openid/login".into(),
    });
    let worker = service.clone();
    tokio::spawn(async move { worker.run().await });
    Ok(routes(service))
}

fn routes(service: Arc<Service>) -> Router<Arc<AppState>> {
    Router::new()
        .route("/auth/steam", get(steam::login))
        .route("/auth/steam/callback", get(steam::callback))
        .route("/api/l4d2/:slug", get(snapshot).post(settings))
        .route(
            "/api/l4d2/:slug/account",
            get(steam::account).delete(steam::unlink),
        )
        .route("/api/l4d2/:slug/admin", get(admin))
        .with_state(service)
}

fn public_address(ip: std::net::IpAddr) -> bool {
    match ip {
        std::net::IpAddr::V4(ip) => {
            let [a, b, c, _] = ip.octets();
            !ip.is_private()
                && !ip.is_loopback()
                && !ip.is_link_local()
                && !ip.is_broadcast()
                && !ip.is_multicast()
                && !ip.is_unspecified()
                && a != 0
                && a < 224
                && !(a == 100 && (64..=127).contains(&b))
                && !(a == 192 && b == 0 && (c == 0 || c == 2))
                && !(a == 198 && (b == 18 || b == 19 || (b == 51 && c == 100)))
                && !(a == 203 && b == 0 && c == 113)
        }
        // Only native global unicast; reject mapped IPv4, ULA, link-local and transition ranges.
        std::net::IpAddr::V6(ip) => {
            let s = ip.segments();
            (s[0] & 0xe000) == 0x2000 && s[0] != 0x2001 && s[0] != 0x2002
        }
    }
}

impl Service {
    fn channel(&self, slug: &str) -> Result<()> {
        if !self.app.channels.contains_key(slug) {
            return Err((StatusCode::NOT_FOUND, "Canal no disponible.".into()));
        }
        Ok(())
    }
    fn authorize(&self, slug: &str, headers: &HeaderMap) -> Result<()> {
        self.channel(slug)?;
        let supplied = headers
            .get(header::AUTHORIZATION)
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.strip_prefix("Bearer "));
        let channel = self
            .app
            .channels
            .get(slug)
            .ok_or_else(|| bad("Canal no disponible."))?;
        if !crate::server::authorized(&channel.panel_token, supplied) {
            return Err((
                StatusCode::UNAUTHORIZED,
                "Se requiere el token de panel del canal, no el de reproducción.".into(),
            ));
        }
        Ok(())
    }
    fn same_origin(&self, headers: &HeaderMap) -> Result<()> {
        let origin = headers.get(header::ORIGIN).and_then(|v| v.to_str().ok());
        let expected = url::Url::parse(&self.app.config.base_url)
            .map_err(|e| failure("BASE_URL inválida.", e))?
            .origin()
            .ascii_serialization();
        if origin != Some(expected.as_str()) {
            return Err((
                StatusCode::FORBIDDEN,
                "Origen de solicitud no permitido.".into(),
            ));
        }
        Ok(())
    }
    async fn run(&self) {
        let mut interval = tokio::time::interval(Duration::from_secs(30));
        interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        loop {
            tokio::select! {
                _ = self.app.shutdown.cancelled() => return,
                _ = interval.tick() => {}
            }
            tokio::select! {
                _ = self.app.shutdown.cancelled() => return,
                result = self.refresh() => {
                    if let Err(error) = result { tracing::error!("[L4D2] Refresh: {}", error.1); }
                }
            }
        }
    }
    async fn refresh(&self) -> Result<()> {
        let configs = sqlx::query_as::<_, (String, String)>(
            "SELECT channel, server FROM l4d2_settings WHERE server <> ''",
        )
        .fetch_all(&self.app.db)
        .await
        .map_err(|e| failure("No se pudo cargar la configuración L4D2.", e))?;
        for (slug, address) in configs {
            let result = match address.parse::<SocketAddr>() {
                Ok(address) if self.servers.contains(&address) => a2s::query(address).await,
                _ => Err(
                    "Servidor fuera de STEAM_A2S_SERVERS. Revisa la configuración del despliegue."
                        .into(),
                ),
            };
            let snapshot = match result {
                Ok(server) => json!({"updated":now(), "server":server, "error":null}),
                Err(error) => {
                    tracing::warn!("[L4D2][{slug}] {error}");
                    json!({"updated":now(), "server":null, "error":error})
                }
            };
            sqlx::query("UPDATE l4d2_settings SET snapshot=$1 WHERE channel=$2 AND server=$3")
                .bind(snapshot.to_string())
                .bind(&slug)
                .bind(&address)
                .execute(&self.app.db)
                .await
                .map_err(|e| failure("No se pudo guardar el estado A2S.", e))?;
        }
        if !self.key.is_empty() {
            let profiles = sqlx::query_as::<_, (String, String)>(
                "SELECT channel, steamid FROM steam_profiles WHERE updated < $1 ORDER BY updated LIMIT 10")
                .bind(now() - 300).fetch_all(&self.app.db).await
                .map_err(|e| failure("No se pudieron cargar los perfiles Steam.", e))?;
            for (slug, steamid) in profiles {
                let update = match self.profile(&steamid).await {
                    Ok(profile) => sqlx::query("UPDATE steam_profiles SET profile=$1, updated=$2 WHERE channel=$3 AND steamid=$4")
                        .bind(profile.to_string()),
                    // Keep the last known name, avatar and history; only flag the failed refresh.
                    Err(error) => {
                        tracing::warn!("[Steam][{slug}] {}", error.1);
                        sqlx::query("UPDATE steam_profiles SET profile=jsonb_set(profile::jsonb, '{error}', to_jsonb($1::text))::text, updated=$2 WHERE channel=$3 AND steamid=$4")
                            .bind(error.1)
                    }
                };
                update.bind(now()).bind(&slug).bind(&steamid)
                    .execute(&self.app.db).await.map_err(|e| failure("No se pudo actualizar el perfil Steam.", e))?;
            }
        }
        sqlx::query("DELETE FROM steam_pending WHERE expires < $1")
            .bind(now())
            .execute(&self.app.db)
            .await
            .map_err(|e| failure("No se pudieron limpiar los inicios de sesión expirados.", e))?;
        sqlx::query("DELETE FROM steam_sessions WHERE expires < $1")
            .bind(now())
            .execute(&self.app.db)
            .await
            .map_err(|e| failure("No se pudieron limpiar las sesiones Steam.", e))?;
        Ok(())
    }
}

async fn snapshot(
    State(service): State<Arc<Service>>,
    Path(slug): Path<String>,
) -> Result<Json<Value>> {
    service.channel(&slug)?;
    let stored =
        sqlx::query_scalar::<_, String>("SELECT snapshot FROM l4d2_settings WHERE channel=$1")
            .bind(&slug)
            .fetch_optional(&service.app.db)
            .await
            .map_err(|e| failure("No se pudo leer A2S.", e))?;
    let snapshot: Value = match stored {
        Some(data) => {
            serde_json::from_str(&data).map_err(|e| failure("Estado A2S guardado inválido.", e))?
        }
        None => json!({}),
    };
    let profiles = sqlx::query_as::<_, (String, String, i64)>(
        "SELECT steamid, profile, updated FROM steam_profiles WHERE channel=$1 ORDER BY steamid",
    )
    .bind(&slug)
    .fetch_all(&service.app.db)
    .await
    .map_err(|e| failure("No se pudieron leer los perfiles.", e))?;
    let mut public_profiles = Vec::new();
    for (steamid, data, updated) in profiles {
        let profile: Value =
            serde_json::from_str(&data).map_err(|e| failure("Perfil guardado inválido.", e))?;
        public_profiles.push(json!({"steamid":steamid,"profile":profile,"updated":updated}));
    }
    let links = sqlx::query_as::<_, (String, String)>(
        "SELECT player_name, steamid FROM l4d2_links WHERE channel=$1 ORDER BY player_name",
    )
    .bind(&slug)
    .fetch_all(&service.app.db)
    .await
    .map_err(|e| failure("No se pudieron leer las vinculaciones.", e))?;
    Ok(Json(json!({
        "snapshot":snapshot, "profiles":public_profiles,
        "links": links.into_iter().map(|(name, steamid)| json!({"name":name,"steamid":steamid,"verified":false})).collect::<Vec<_>>(),
        "steam_enabled": !service.key.is_empty(), "poll_seconds":30
    })))
}

async fn admin(
    State(service): State<Arc<Service>>,
    Path(slug): Path<String>,
    headers: HeaderMap,
) -> Result<Json<Value>> {
    service.authorize(&slug, &headers)?;
    let server =
        sqlx::query_scalar::<_, String>("SELECT server FROM l4d2_settings WHERE channel=$1")
            .bind(&slug)
            .fetch_optional(&service.app.db)
            .await
            .map_err(|e| failure("No se pudo leer el servidor configurado.", e))?;
    Ok(Json(
        json!({"server":server, "allowed_servers":service.servers.iter().map(ToString::to_string).collect::<Vec<_>>()}),
    ))
}

#[derive(Deserialize)]
#[serde(tag = "action", rename_all = "snake_case", deny_unknown_fields)]
enum Change {
    Server { address: String },
    Link { name: String, steamid: String },
    Unlink { name: String },
}

async fn settings(
    State(service): State<Arc<Service>>,
    Path(slug): Path<String>,
    headers: HeaderMap,
    Json(change): Json<Change>,
) -> Result<Response> {
    service.authorize(&slug, &headers)?;
    service.same_origin(&headers)?;
    let result = match change {
        Change::Server { address } => {
            if !address.is_empty() && !service.servers.iter().any(|v| v.to_string() == address) {
                return Err(bad(
                    "Selecciona un servidor permitido en STEAM_A2S_SERVERS.",
                ));
            }
            let mut tx = service
                .app
                .db
                .begin()
                .await
                .map_err(|e| failure("No se pudo cambiar el servidor.", e))?;
            sqlx::query("DELETE FROM l4d2_links WHERE channel=$1")
                .bind(&slug)
                .execute(&mut *tx)
                .await
                .map_err(|e| {
                    failure(
                        "No se pudieron limpiar las asociaciones del servidor anterior.",
                        e,
                    )
                })?;
            sqlx::query("INSERT INTO l4d2_settings(channel,server) VALUES($1,$2) ON CONFLICT(channel) DO UPDATE SET server=$2,snapshot='{}'")
                .bind(&slug).bind(address).execute(&mut *tx).await
                .map_err(|e| failure("No se pudo guardar el servidor.", e))?;
            tx.commit()
                .await
                .map_err(|e| failure("No se pudo confirmar el servidor.", e))?;
            return Ok(StatusCode::NO_CONTENT.into_response());
        }
        Change::Link { name, steamid } => {
            if name.is_empty() || name.chars().count() > 256 || !steam::valid_id(&steamid) {
                return Err(bad("Nombre de jugador o SteamID inválido."));
            }
            let snapshot = sqlx::query_scalar::<_, String>(
                "SELECT snapshot FROM l4d2_settings WHERE channel=$1",
            )
            .bind(&slug)
            .fetch_optional(&service.app.db)
            .await
            .map_err(|e| failure("No se pudo comprobar el jugador.", e))?
            .ok_or_else(|| bad("Consulta primero el servidor."))?;
            let snapshot: Value =
                serde_json::from_str(&snapshot).map_err(|e| failure("Estado A2S inválido.", e))?;
            let recent = snapshot["updated"]
                .as_i64()
                .is_some_and(|t| now() - t <= 90);
            let matches = snapshot["server"]["players"]
                .as_array()
                .map_or(0, |players| {
                    players
                        .iter()
                        .filter(|p| p["name"].as_str() == Some(&name))
                        .count()
                });
            if !recent || matches != 1 {
                return Err(bad("El nombre debe aparecer una sola vez en una consulta A2S reciente. No se pueden distinguir nombres duplicados."));
            }
            let exists: bool = sqlx::query_scalar(
                "SELECT EXISTS(SELECT 1 FROM steam_profiles WHERE channel=$1 AND steamid=$2)",
            )
            .bind(&slug)
            .bind(&steamid)
            .fetch_one(&service.app.db)
            .await
            .map_err(|e| failure("No se pudo comprobar el perfil.", e))?;
            if !exists {
                return Err(bad(
                    "Ese usuario debe iniciar sesión con Steam en este canal primero.",
                ));
            }
            sqlx::query("INSERT INTO l4d2_links(channel,player_name,steamid) VALUES($1,$2,$3) ON CONFLICT(channel,player_name) DO UPDATE SET steamid=$3")
                .bind(&slug).bind(name).bind(steamid).execute(&service.app.db).await
        }
        Change::Unlink { name } => {
            sqlx::query("DELETE FROM l4d2_links WHERE channel=$1 AND player_name=$2")
                .bind(&slug)
                .bind(name)
                .execute(&service.app.db)
                .await
        }
    };
    result.map_err(|e| failure("No se pudo guardar la configuración L4D2.", e))?;
    Ok(StatusCode::NO_CONTENT.into_response())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn only_public_literal_targets_are_allowed() {
        for ip in [
            "127.0.0.1",
            "10.0.0.1",
            "192.168.1.1",
            "169.254.169.254",
            "100.64.1.1",
            "0.1.2.3",
            "224.0.0.1",
            "240.1.1.1",
            "192.0.2.1",
            "198.51.100.1",
            "203.0.113.1",
            "::1",
            "::ffff:127.0.0.1",
            "fc00::1",
            "2001:db8::1",
            "2002:7f00:1::",
        ] {
            assert!(!public_address(ip.parse().unwrap()), "{ip}");
        }
        assert!(public_address("8.8.8.8".parse().unwrap()));
        assert!(public_address("2606:4700::1111".parse().unwrap()));
    }
}
