#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

mod auth;
mod channel;
mod commands;
mod config;
mod cooldown;
mod db;
mod kick;
mod login;
mod queue;
mod server;
mod state;
mod stats;
mod tts;

use axum::{
    body::Bytes,
    extract::{Query, State},
    http::{HeaderMap, StatusCode},
};
use dashmap::DashMap;
use socketioxide::SocketIo;
use state::{AppState, GlobalConfig};
use std::{collections::HashMap, sync::Arc};
use tower_http::services::ServeDir;
use tracing::info;
use tracing_subscriber::EnvFilter;

// ─── Webhook de Kick (EventSub) ───────────────────────────────────────────────

async fn kick_webhook_get(
    Query(params): Query<HashMap<String, String>>,
) -> (StatusCode, String) {
    if let Some(ch) = params.get("hub.challenge").or_else(|| params.get("challenge")) {
        info!("[Webhook] Verificación GET OK");
        return (StatusCode::OK, ch.clone());
    }
    (StatusCode::OK, "ok".into())
}

async fn kick_webhook(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    body: Bytes,
) -> (StatusCode, String) {
    let body_str = String::from_utf8_lossy(&body);
    info!("[Webhook] ← {:.500}", body_str);

    let json: serde_json::Value = match serde_json::from_slice(&body) {
        Ok(j)  => j,
        Err(_) => return (StatusCode::OK, "ok".into()),
    };

    // Challenge de verificación
    if let Some(ch) = json.get("challenge").and_then(|c| c.as_str()) {
        info!("[Webhook] Verificación OK");
        return (StatusCode::OK, ch.to_string());
    }

    // Determinar tipo de evento
    let event_type = headers.get("Kick-Event-Type")
        .or_else(|| headers.get("Kick-Eventsub-Message-Type"))
        .and_then(|v| v.to_str().ok())
        .or_else(|| json["type"].as_str())
        .or_else(|| json["event_type"].as_str())
        .unwrap_or("unknown")
        .to_string();

    info!("[Webhook] event={event_type}");

    // Rutear al canal correcto por broadcaster_user_id
    let broadcaster_id = json["event"]["broadcaster_user_id"].as_u64()
        .or_else(|| json["broadcaster_user_id"].as_u64())
        .or_else(|| json["data"]["broadcaster_user_id"].as_u64());

    let ch_arc = broadcaster_id
        .and_then(|bid| state.user_id_to_slug.get(&bid).map(|s| s.clone()))
        .and_then(|slug| state.channels.get(&slug).map(|c| c.clone()));

    let Some(ch) = ch_arc else {
        info!("[Webhook] Canal no encontrado para broadcaster_id={:?}", broadcaster_id);
        return (StatusCode::OK, "ok".into());
    };

    let ev = json.get("event").unwrap_or(&json);

    if event_type.contains("chat") || event_type.contains("message") {
        let username = ev["sender"]["username"].as_str()
            .or_else(|| ev["sender"]["slug"].as_str())
            .unwrap_or("?").to_string();
        let content = ev["content"].as_str()
            .or_else(|| ev["message"]["content"].as_str())
            .unwrap_or("").trim().to_string();
        if !content.is_empty() {
            info!("[CHAT-WH][{}] {username}: {content}", ch.slug);
            server::ns_emit(&state, &ch.slug, "chatMessage",
                serde_json::json!({"user": &username, "content": &content}));
            commands::handle(&username, &content, &ch, &state).await;
        }
    } else if event_type.contains("follow") {
        let username = ev["follower"]["username"].as_str()
            .or_else(|| ev["username"].as_str())
            .unwrap_or("alguien").to_string();
        info!("[Webhook][{}] Follow: {username}", ch.slug);
        ch.tts_tx.send(tts::TtsQueueItem {
            text:  format!("¡Gracias por el follow, {username}!"),
            voice: "dalia".into(),
        }).ok();
        server::ns_emit(&state, &ch.slug, "kickAlert",
            serde_json::json!({"type":"follow","username":&username}));
    } else if event_type.contains("subscription") || event_type.contains("sub") {
        let username = ev["subscriber"]["username"].as_str()
            .or_else(|| ev["user"]["username"].as_str())
            .unwrap_or("alguien");
        let months = ev["months"].as_u64().unwrap_or(1);
        info!("[Webhook][{}] Sub: {username} ({months}m)", ch.slug);
        let msg = if months > 1 { format!("¡{username} se resuscribió por {months} meses!") }
                  else { format!("¡{username} se suscribió al canal!") };
        ch.tts_tx.send(tts::TtsQueueItem { text: msg.clone(), voice: "dalia".into() }).ok();
        server::ns_emit(&state, &ch.slug, "kickAlert",
            serde_json::json!({"type":"sub","username":username,"months":months,"message":msg}));
    }

    (StatusCode::OK, "ok".into())
}

// ─── Entrypoint ───────────────────────────────────────────────────────────────

#[tokio::main]
async fn main() {
    // Modo login local (para desarrollo)
    if std::env::args().any(|a| a == "--login") {
        #[cfg(windows)]
        unsafe {
            extern "system" { fn AllocConsole() -> i32; }
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
        client_id:     std::env::var("KICK_CLIENT_ID").unwrap_or_default(),
        client_secret: std::env::var("KICK_CLIENT_SECRET").unwrap_or_default(),
        port:          std::env::var("PORT").ok().and_then(|v| v.parse().ok()).unwrap_or(3000),
        overlay_dir:   std::env::var("OVERLAY_DIR").unwrap_or_else(|_| "../overlay".into()),
        base_url:      std::env::var("BASE_URL")
            .or_else(|_| std::env::var("RENDER_EXTERNAL_URL"))
            .unwrap_or_else(|_| "http://localhost:3000".into()),
        tts_cache_dir: std::env::var("TTS_CACHE_DIR").unwrap_or_else(|_| "/tmp/tts_cache".into()),
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
    let db = sqlx::postgres::PgPoolOptions::new()
        .max_connections(5)
        .connect(&db_url)
        .await
        .unwrap_or_else(|e| { eprintln!("[FATAL] No se pudo conectar a PostgreSQL: {e}"); std::process::exit(1); });

    // Ejecutar migraciones
    db::run_migrations(&db).await;
    info!("Base de datos lista");

    // Socket.IO
    let (layer, io_inner) = SocketIo::new_layer();

    // HTTP client
    let http = reqwest::Client::builder()
        .user_agent("DaiBot/1.0")
        .timeout(std::time::Duration::from_secs(30))
        .build()
        .expect("reqwest client");

    // Estado global
    let state = Arc::new(AppState {
        config,
        http,
        io: io_inner,
        db,
        channels:        Arc::new(DashMap::new()),
        user_id_to_slug: Arc::new(DashMap::new()),
    });

    // Registrar el namespace Socket.IO único (rooms por canal)
    server::setup(&state.io.clone(), state.clone());

    // Cargar todos los canales registrados
    let registered = db::load_all_channels(&state.db).await;
    info!("Canales registrados: {}", registered.len());
    for row in registered {
        channel::start_channel(row, state.clone()).await;
    }

    // Overlay
    let overlay_dir = resolve_overlay_dir(&state.config.overlay_dir);
    info!("Overlay: {}", overlay_dir.display());

    // Router
    let app = axum::Router::new()
        .route("/", axum::routing::get(auth::start_oauth))
        .route("/auth/kick", axum::routing::get(auth::redirect_to_kick))
        .route("/auth/callback", axum::routing::get(auth::handle_callback))
        .route("/kick_webhook",
            axum::routing::post(kick_webhook).get(kick_webhook_get))
        .with_state(state.clone())
        .fallback_service(ServeDir::new(&overlay_dir))
        .layer(layer);

    let addr = format!("0.0.0.0:{}", state.config.port);

    // Liberar puerto si hay instancia anterior (solo Windows, desarrollo)
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        let _ = std::process::Command::new("powershell")
            .args(["-NoProfile", "-Command", &format!(
                "Get-NetTCPConnection -LocalPort {} -ErrorAction SilentlyContinue \
                 | ForEach-Object {{ Stop-Process -Id $_.OwningProcess -Force -ErrorAction SilentlyContinue }}",
                state.config.port
            )])
            .creation_flags(0x08000000)
            .output();
        std::thread::sleep(std::time::Duration::from_millis(500));
    }

    let listener = tokio::net::TcpListener::bind(&addr).await
        .unwrap_or_else(|e| { eprintln!("[FATAL] Puerto {}: {e}", state.config.port); std::process::exit(1); });

    info!("DaiBot corriendo  → http://localhost:{}", state.config.port);
    info!("Registro          → http://localhost:{}/", state.config.port);

    axum::serve(listener, app).await
        .unwrap_or_else(|e| { eprintln!("[FATAL] Servidor: {e}"); std::process::exit(1); });
}

fn resolve_overlay_dir(configured: &str) -> std::path::PathBuf {
    let p = std::path::Path::new(configured);
    if p.is_absolute() && p.exists() { return p.to_path_buf(); }
    if let Ok(exe) = std::env::current_exe() {
        let candidate = exe.parent().map(|d| d.join(configured)).unwrap_or_default();
        if candidate.exists() { return candidate; }
        for ancestor in exe.ancestors().skip(1) {
            let c = ancestor.join("overlay");
            if c.join("pixel.html").exists() { return c; }
        }
    }
    p.to_path_buf()
}

pub(crate) fn find_dotenv_path() -> Option<std::path::PathBuf> {
    if let Ok(exe) = std::env::current_exe() {
        for ancestor in exe.ancestors().skip(1) {
            let candidate = ancestor.join(".env");
            if candidate.exists() { return Some(candidate); }
        }
    }
    let p = std::path::Path::new(".env");
    if p.exists() { Some(p.to_path_buf()) } else { None }
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
