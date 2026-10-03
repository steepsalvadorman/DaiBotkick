#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]
mod auth;
mod channel;
mod commands;
mod config;
mod cooldown;
mod db;
#[cfg(test)]
mod integration_tests;
mod kick;
mod login;
mod queue;
mod server;
mod state;
mod stats;
#[cfg(test)]
mod test_support;
mod tts;
mod webhook;

use axum::{extract::State, http::StatusCode};
use dashmap::DashMap;
use socketioxide::SocketIo;
use state::{AppState, GlobalConfig};
use std::sync::Arc;
use tokio::sync::{Mutex, Semaphore};
use tokio_util::sync::CancellationToken;
use tower_http::services::ServeDir;
use tracing::info;
use tracing_subscriber::EnvFilter;

async fn ready(State(state): State<Arc<AppState>>) -> StatusCode {
    if state.shutdown.is_cancelled() {
        return StatusCode::SERVICE_UNAVAILABLE;
    }
    match tokio::time::timeout(
        std::time::Duration::from_secs(2),
        sqlx::query("SELECT 1").execute(&state.db),
    )
    .await
    {
        Ok(Ok(_)) => StatusCode::OK,
        _ => StatusCode::SERVICE_UNAVAILABLE,
    }
}

async fn metrics(State(state): State<Arc<AppState>>) -> String {
    use std::sync::atomic::Ordering::Relaxed;
    let pending_tts: usize = state
        .channels
        .iter()
        .map(|channel| channel.tts_tx.max_capacity() - channel.tts_tx.capacity())
        .sum();
    let sockets = state.io.sockets().map_or(0, |sockets| sockets.len());
    format!("daibot_channels {}\ndaibot_sockets {}\ndaibot_tts_pending {}\ndaibot_webhook_rejected_total {}\ndaibot_webhook_received_total {}\ndaibot_webhook_duplicates_total {}\ndaibot_webhook_processed_total {}\n",
        state.channels.len(), sockets, pending_tts, state.metrics.webhook_rejected.load(Relaxed),
        state.metrics.webhook_received.load(Relaxed), state.metrics.webhook_duplicates.load(Relaxed), state.metrics.webhook_processed.load(Relaxed))
}
async fn shutdown_signal() {
    #[cfg(unix)]
    {
        let mut term = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
            .expect("SIGTERM");
        tokio::select! { _ = tokio::signal::ctrl_c() => {}, _ = term.recv() => {} }
    }
    #[cfg(not(unix))]
    {
        tokio::signal::ctrl_c().await.expect("Ctrl+C");
    }
}

#[tokio::main]
async fn main() {
    // Modo login local (para desarrollo)
    if std::env::args().any(|a| a == "--login") {
        #[cfg(windows)]
        unsafe {
            extern "system" {
                fn AllocConsole() -> i32;
            }
            AllocConsole();
        }
        login::run_and_exit().await;
        return;
    }

    // Cargar .env (solo para desarrollo local)
    if let Some(path) = find_dotenv_path() {
        let _ = dotenvy::from_path(&path);
    } else {
        let _ = dotenvy::dotenv();
    }

    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .init();

    // Configuración global (solo credenciales de la app, no del canal)
    let config = GlobalConfig {
        client_id: std::env::var("KICK_CLIENT_ID").unwrap_or_default(),
        client_secret: std::env::var("KICK_CLIENT_SECRET").unwrap_or_default(),
        port: std::env::var("PORT")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(3000),
        overlay_dir: std::env::var("OVERLAY_DIR").unwrap_or_else(|_| "../overlay".into()),
        base_url: std::env::var("BASE_URL")
            .or_else(|_| std::env::var("RENDER_EXTERNAL_URL"))
            .unwrap_or_else(|_| "http://localhost:3000".into()),
        tts_cache_dir: std::env::var("TTS_CACHE_DIR").unwrap_or_else(|_| {
            std::env::temp_dir()
                .join("daibot_tts")
                .to_string_lossy()
                .into_owned()
        }),
    };

    if config.client_id.is_empty() || config.client_secret.is_empty() {
        eprintln!("[FATAL] Faltan KICK_CLIENT_ID o KICK_CLIENT_SECRET.");
        std::process::exit(1);
    }

    // Base de datos PostgreSQL
    let db_url = std::env::var("DATABASE_URL").unwrap_or_else(|_| {
        eprintln!("[FATAL] DATABASE_URL no configurada.");
        std::process::exit(1);
    });
    let connect_opts = db_url
        .parse::<sqlx::postgres::PgConnectOptions>()
        .unwrap_or_else(|e| {
            eprintln!("[FATAL] DATABASE_URL inválida: {e}");
            std::process::exit(1);
        })
        .statement_cache_capacity(0); // requerido para PgBouncer (transaction pooler)
    let db = sqlx::postgres::PgPoolOptions::new()
        .max_connections(5)
        .connect_with(connect_opts)
        .await
        .unwrap_or_else(|e| {
            eprintln!("[FATAL] No se pudo conectar a PostgreSQL: {e}");
            std::process::exit(1);
        });

    // Ejecutar migraciones
    db::run_migrations(&db)
        .await
        .unwrap_or_else(|e| fatal(&format!("Migraciones: {e}")));
    info!("Base de datos lista");

    // Socket.IO
    let (layer, io_inner) = SocketIo::new_layer();

    // HTTP client
    let http = reqwest::Client::builder()
        .user_agent("GorilinRix/1.0")
        .timeout(std::time::Duration::from_secs(30))
        .build()
        .expect("reqwest client");

    // Estado global
    let kick_endpoints = kick::Endpoints::default();
    let webhook_key = webhook::fetch_public_key(&http, &kick_endpoints.api).await;
    let state = Arc::new(AppState {
        config,
        http,
        kick_endpoints,
        io: io_inner,
        db,
        channels: Arc::new(DashMap::new()),
        user_id_to_slug: Arc::new(DashMap::new()),
        channel_lock: Mutex::new(()),
        shutdown: CancellationToken::new(),
        tts_slots: Arc::new(Semaphore::new(2)),
        webhook_key,
        metrics: Arc::new(state::Metrics::default()),
    });

    // Registrar el namespace Socket.IO único (rooms por canal)
    server::setup(&state.io.clone(), state.clone());

    // Cargar todos los canales registrados
    let registered = db::load_all_channels(&state.db)
        .await
        .unwrap_or_else(|e| fatal(&format!("Carga de canales: {e}")));
    info!("Canales registrados: {}", registered.len());
    for row in registered {
        channel::start_channel(row, state.clone())
            .await
            .unwrap_or_else(|e| fatal(&e));
    }

    // Overlay
    let inbox = state.clone();
    tokio::spawn(async move {
        webhook::worker(inbox).await;
    });
    let overlay_dir = resolve_overlay_dir(&state.config.overlay_dir);
    info!("Overlay: {}", overlay_dir.display());

    // Router
    let app = axum::Router::new()
        .route("/", axum::routing::get(auth::start_oauth))
        .route("/auth/kick", axum::routing::get(auth::redirect_to_kick))
        .route("/auth/callback", axum::routing::get(auth::handle_callback))
        .route("/kick_webhook", axum::routing::post(webhook::receive))
        .route("/healthz", axum::routing::get(|| async { StatusCode::OK }))
        .route("/readyz", axum::routing::get(ready))
        .route("/metrics", axum::routing::get(metrics))
        .layer(axum::extract::DefaultBodyLimit::max(256 * 1024))
        .with_state(state.clone())
        .fallback_service(ServeDir::new(&overlay_dir))
        .layer(axum::middleware::from_fn(response_headers))
        .layer(layer);

    let addr = format!("0.0.0.0:{}", state.config.port);

    let listener = tokio::net::TcpListener::bind(&addr)
        .await
        .unwrap_or_else(|e| {
            eprintln!("[FATAL] Puerto {}: {e}", state.config.port);
            std::process::exit(1);
        });

    info!(
        "GorilinRix corriendo  → http://localhost:{}",
        state.config.port
    );
    info!(
        "Registro          → http://localhost:{}/",
        state.config.port
    );

    let shutdown = state.shutdown.clone();
    let signal = shutdown.clone();
    tokio::spawn(async move {
        shutdown_signal().await;
        signal.cancel();
    });
    axum::serve(listener, app)
        .with_graceful_shutdown(async move { shutdown.cancelled().await })
        .await
        .unwrap_or_else(|e| {
            eprintln!("[FATAL] Servidor: {e}");
            std::process::exit(1);
        });
}

async fn response_headers(
    request: axum::extract::Request,
    next: axum::middleware::Next,
) -> axum::response::Response {
    let private = request.uri().path().starts_with("/auth");
    let mut response = next.run(request).await;
    response
        .headers_mut()
        .insert("x-content-type-options", "nosniff".parse().unwrap());
    response.headers_mut().insert(
        "referrer-policy",
        if private {
            "no-referrer"
        } else {
            "strict-origin-when-cross-origin"
        }
        .parse()
        .unwrap(),
    );
    if private {
        response
            .headers_mut()
            .insert("cache-control", "no-store".parse().unwrap());
    }
    response
}

fn resolve_overlay_dir(configured: &str) -> std::path::PathBuf {
    let p = std::path::Path::new(configured);
    if p.is_absolute() && p.exists() {
        return p.to_path_buf();
    }
    if let Ok(exe) = std::env::current_exe() {
        let candidate = exe.parent().map(|d| d.join(configured)).unwrap_or_default();
        if candidate.exists() {
            return candidate;
        }
        for ancestor in exe.ancestors().skip(1) {
            let c = ancestor.join("overlay");
            if c.join("pixel.html").exists() {
                return c;
            }
        }
    }
    p.to_path_buf()
}

pub(crate) fn find_dotenv_path() -> Option<std::path::PathBuf> {
    if let Ok(exe) = std::env::current_exe() {
        for ancestor in exe.ancestors().skip(1) {
            let candidate = ancestor.join(".env");
            if candidate.exists() {
                return Some(candidate);
            }
        }
    }
    let p = std::path::Path::new(".env");
    if p.exists() {
        Some(p.to_path_buf())
    } else {
        None
    }
}

pub(crate) fn load_dotenv() {
    if let Some(path) = find_dotenv_path() {
        let _ = dotenvy::from_path(&path);
    } else {
        let _ = dotenvy::dotenv();
    }
}

pub(crate) fn fatal(msg: &str) -> ! {
    eprintln!("\n[FATAL] {msg}\n");
    std::process::exit(1);
}
