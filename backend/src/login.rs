use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine as _};
use rand::RngCore;
use sha2::{Digest, Sha256};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

const REDIRECT_URI: &str = "http://localhost:3001/callback";
const AUTH_URL: &str = "https://id.kick.com/oauth/authorize";
const TOKEN_URL: &str = "https://id.kick.com/oauth/token";
const SCOPES: &str = "user:read channel:read channel:update chat:write events:subscribe";

/// Lanzado con --login: hace el flujo OAuth y cierra.
pub async fn run_and_exit() {
    match run_flow().await {
        Ok(_) => {
            println!();
            println!("Todo listo. Ya puedes lanzar DaiBot.");
            println!();
            println!("Presiona Enter para cerrar...");
            let mut buf = String::new();
            let _ = std::io::stdin().read_line(&mut buf);
        }
        Err(e) => crate::fatal(&e),
    }
}

/// Primer arranque automático: hace el flujo OAuth y continúa el bot.
pub async fn run_first_time() {
    println!("╔══════════════════════════════════════════════╗");
    println!("║         BIENVENIDO A DAIBOT                  ║");
    println!("║  Primera vez detectada — vamos a configurar  ║");
    println!("╚══════════════════════════════════════════════╝");
    println!();
    println!("Se abrirá tu navegador para que inicies sesión");
    println!("con tu cuenta de Kick. Sigue las instrucciones.");
    println!();

    match run_flow().await {
        Ok(_) => {
            // Recargar .env con los nuevos tokens
            crate::load_dotenv();
            println!();
            println!("Configuración completa. Iniciando el bot...");
            println!();
        }
        Err(e) => crate::fatal(&e),
    }
}

// ── Flujo OAuth completo ──────────────────────────────────────────────────────

async fn run_flow() -> Result<(), String> {
    crate::load_dotenv();

    let client_id = std::env::var("KICK_CLIENT_ID").unwrap_or_default();
    let client_secret = std::env::var("KICK_CLIENT_SECRET").unwrap_or_default();

    if client_id.is_empty() || client_secret.is_empty() {
        return Err(
            "Faltan KICK_CLIENT_ID o KICK_CLIENT_SECRET.\n\
             Contacta al soporte de DaiBot."
                .to_string(),
        );
    }

    // PKCE
    let code_verifier = random_base64url(32);
    let code_challenge = sha256_base64url(&code_verifier);
    let state = random_hex(16);

    let query = url::form_urlencoded::Serializer::new(String::new())
        .append_pair("client_id", &client_id)
        .append_pair("redirect_uri", REDIRECT_URI)
        .append_pair("response_type", "code")
        .append_pair("scope", SCOPES)
        .append_pair("state", &state)
        .append_pair("code_challenge", &code_challenge)
        .append_pair("code_challenge_method", "S256")
        .finish();
    let auth_url = format!("{AUTH_URL}?{query}");

    open_browser(&auth_url);

    // En Windows sin consola mostrar dialog con URL de fallback
    #[cfg(windows)]
    msg_dialog(
        &format!(
            "Se ha abierto tu navegador para iniciar sesión en Kick.\n\
             Acepta los permisos y vuelve aquí cuando termines.\n\n\
             Si el navegador NO se abrió, copia esta dirección:\n\n{}",
            auth_url
        ),
        "DaiBot — Conectar con Kick",
        0x40, // MB_ICONINFORMATION
    );

    println!("Abriendo navegador...");
    println!("Si no se abre, copia esta URL:");
    println!("  {auth_url}");
    println!();

    let code = wait_callback(&state).await?;

    println!("Obteniendo tokens...");
    let http = reqwest::Client::new();
    let tokens = exchange_code(&http, &code, &code_verifier, &client_id, &client_secret).await?;

    let access  = tokens["access_token"].as_str().unwrap_or_default().to_string();
    let refresh = tokens["refresh_token"].as_str().unwrap_or_default().to_string();
    let expires = tokens["expires_in"].as_u64().unwrap_or(7200);

    save_tokens(&access, &refresh, expires);

    // Obtener nombre del canal automáticamente
    fetch_and_save_channel_name(&http, &access).await;

    Ok(())
}

// ── Captura automática del nombre de canal ────────────────────────────────────

async fn fetch_and_save_channel_name(http: &reqwest::Client, token: &str) {
    println!("Obteniendo datos de tu canal de Kick...");

    let slug = try_fetch_slug(http, token).await.unwrap_or_else(|| {
        #[cfg(windows)]
        {
            input_dialog(
                "No se pudo detectar tu nombre de canal en Kick.\n\
                 Escribe tu nombre de usuario (ejemplo: seniordai):",
                "DaiBot — Configuración",
            ).unwrap_or_default()
        }
        #[cfg(not(windows))]
        {
            println!();
            println!("No se pudo detectar tu nombre de canal automáticamente.");
            print!("Escribe tu nombre de canal en Kick (ej: seniordai): ");
            use std::io::Write;
            std::io::stdout().flush().ok();
            let mut input = String::new();
            std::io::stdin().read_line(&mut input).ok();
            input.trim().to_lowercase()
        }
    });

    if !slug.is_empty() {
        save_env_key("CHANNEL_NAME", &slug);
        println!("  Canal detectado: {slug}");
    }
}

async fn try_fetch_slug(http: &reqwest::Client, token: &str) -> Option<String> {
    // Intento 1: endpoint de usuario autenticado
    if let Some(slug) = fetch_from_user_endpoint(http, token).await {
        return Some(slug);
    }
    // Intento 2: endpoint de canal propio
    fetch_from_channel_endpoint(http, token).await
}

async fn fetch_from_user_endpoint(http: &reqwest::Client, token: &str) -> Option<String> {
    let resp = http
        .get("https://api.kick.com/public/v1/user")
        .header("Authorization", format!("Bearer {token}"))
        .send()
        .await
        .ok()?;

    if !resp.status().is_success() {
        return None;
    }

    let json: serde_json::Value = resp.json().await.ok()?;

    // La API puede devolver el slug en distintos campos según la versión
    json["data"]["slug"].as_str()
        .or_else(|| json["data"]["username"].as_str())
        .or_else(|| json["data"]["name"].as_str())
        .or_else(|| json["slug"].as_str())
        .or_else(|| json["username"].as_str())
        .map(|s| s.to_lowercase())
}

async fn fetch_from_channel_endpoint(http: &reqwest::Client, token: &str) -> Option<String> {
    // Llamar al endpoint de canales del usuario autenticado
    let resp = http
        .get("https://api.kick.com/public/v1/channels")
        .header("Authorization", format!("Bearer {token}"))
        .send()
        .await
        .ok()?;

    if !resp.status().is_success() {
        return None;
    }

    let json: serde_json::Value = resp.json().await.ok()?;
    json["data"].as_array()?
        .first()?["slug"]
        .as_str()
        .map(|s| s.to_lowercase())
}

// ── .env helpers ──────────────────────────────────────────────────────────────

fn save_tokens(access: &str, refresh: &str, expires_in: u64) {
    let expires_at = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
        + expires_in;

    save_env_key("KICK_ACCESS_TOKEN", access);
    save_env_key("KICK_REFRESH_TOKEN", refresh);
    save_env_key("KICK_TOKEN_EXPIRES", &expires_at.to_string());
}

fn save_env_key(key: &str, value: &str) {
    let path = crate::find_dotenv_path().unwrap_or_else(|| {
        std::env::current_exe()
            .ok()
            .and_then(|p| p.parent().map(|d| d.join(".env")))
            .unwrap_or_else(|| std::path::PathBuf::from(".env"))
    });

    let content = std::fs::read_to_string(&path).unwrap_or_default();
    let updated = set_key(content, key, value);
    let _ = std::fs::write(&path, format!("{}\n", updated.trim_end()));
}

fn set_key(content: String, key: &str, value: &str) -> String {
    let escaped = value.replace('\\', "\\\\").replace('"', "\\\"");
    let line = format!("{key}=\"{escaped}\"");
    let prefix = format!("{key}=");

    if content.lines().any(|l| l.starts_with(&prefix)) {
        content
            .lines()
            .map(|l| if l.starts_with(&prefix) { line.as_str() } else { l })
            .collect::<Vec<_>>()
            .join("\n")
    } else if content.is_empty() {
        line
    } else {
        format!("{content}\n{line}")
    }
}

// ── OAuth: callback HTTP ──────────────────────────────────────────────────────

async fn wait_callback(expected_state: &str) -> Result<String, String> {
    let listener = TcpListener::bind("127.0.0.1:3001").await.map_err(|e| {
        format!("No se pudo escuchar en puerto 3001: {e}")
    })?;

    println!("Esperando autorización en el navegador...");

    loop {
        let (mut socket, _) = listener.accept().await.map_err(|e| e.to_string())?;

        let mut buf = vec![0u8; 8192];
        let n = socket.read(&mut buf).await.map_err(|e| e.to_string())?;
        let request = String::from_utf8_lossy(&buf[..n]);

        let path = request
            .lines()
            .next()
            .and_then(|l| l.split_whitespace().nth(1))
            .unwrap_or("");

        if !path.starts_with("/callback") {
            continue;
        }

        let qs = path.find('?').map(|i| &path[i + 1..]).unwrap_or("");
        let params: std::collections::HashMap<_, _> =
            url::form_urlencoded::parse(qs.as_bytes()).collect();

        if let Some(err) = params.get("error") {
            html_reply(&mut socket, &format!(
                "<h2>Error: {err}</h2><p>Puedes cerrar esta pestaña.</p>"
            )).await;
            return Err(format!("OAuth rechazado: {err}"));
        }

        let state_recv = params.get("state").map(|v| v.as_ref()).unwrap_or("");
        if state_recv != expected_state {
            html_reply(&mut socket, "<h2>Estado inválido</h2>").await;
            continue;
        }

        if let Some(code) = params.get("code") {
            html_reply(
                &mut socket,
                "<h2 style='color:#53fc18'>&#x2705; ¡Autorizado correctamente!</h2>\
                 <p>Puedes cerrar esta pestaña y volver al bot.</p>",
            ).await;
            println!("Autorización recibida.");
            return Ok(code.to_string());
        }
    }
}

async fn html_reply(socket: &mut tokio::net::TcpStream, body: &str) {
    let html = format!(
        "<html><head><meta charset='utf-8'></head>\
         <body style='font-family:sans-serif;text-align:center;padding:40px;\
         background:#0a0a0a;color:#fff'>{body}</body></html>"
    );
    let response = format!(
        "HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\n\
         Content-Length: {}\r\nConnection: close\r\n\r\n{}",
        html.len(), html
    );
    socket.write_all(response.as_bytes()).await.ok();
}

// ── OAuth: intercambio de código ──────────────────────────────────────────────

async fn exchange_code(
    client: &reqwest::Client,
    code: &str,
    code_verifier: &str,
    client_id: &str,
    client_secret: &str,
) -> Result<serde_json::Value, String> {
    let params = [
        ("grant_type", "authorization_code"),
        ("client_id", client_id),
        ("client_secret", client_secret),
        ("redirect_uri", REDIRECT_URI),
        ("code", code),
        ("code_verifier", code_verifier),
    ];

    let resp = client
        .post(TOKEN_URL)
        .form(&params)
        .send()
        .await
        .map_err(|e| format!("Error de red: {e}"))?;

    let status = resp.status();
    if !status.is_success() {
        let text = resp.text().await.unwrap_or_default();
        return Err(format!("Error {status}: {text}"));
    }

    let data: serde_json::Value = resp.json().await
        .map_err(|e| format!("Respuesta inválida: {e}"))?;

    if data["access_token"].is_null() {
        return Err(format!("Respuesta sin access_token: {data}"));
    }

    Ok(data)
}

// ── PKCE / aleatorio ──────────────────────────────────────────────────────────

fn random_base64url(n: usize) -> String {
    let mut bytes = vec![0u8; n];
    rand::thread_rng().fill_bytes(&mut bytes);
    URL_SAFE_NO_PAD.encode(&bytes)
}

fn sha256_base64url(s: &str) -> String {
    URL_SAFE_NO_PAD.encode(Sha256::digest(s.as_bytes()))
}

fn random_hex(n: usize) -> String {
    let mut bytes = vec![0u8; n];
    rand::thread_rng().fill_bytes(&mut bytes);
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn open_browser(url: &str) {
    #[cfg(windows)]
    {
        // rundll32 abre el navegador predeterminado del sistema sin problema de & en la URL
        let ok = std::process::Command::new("rundll32")
            .args(["url.dll,FileProtocolHandler", url])
            .spawn()
            .is_ok();
        // Fallback: explorer.exe también sabe abrir URLs
        if !ok {
            let _ = std::process::Command::new("explorer").arg(url).spawn();
        }
    }
    #[cfg(target_os = "macos")]
    let _ = std::process::Command::new("open").arg(url).spawn();
    #[cfg(target_os = "linux")]
    {
        // Probar los abridos más comunes en orden
        for cmd in ["xdg-open", "sensible-browser", "firefox", "chromium-browser"] {
            if std::process::Command::new(cmd).arg(url).spawn().is_ok() {
                break;
            }
        }
    }
}

// ── Dialogs nativos de Windows (sin dependencias extra) ───────────────────────

#[cfg(windows)]
fn msg_dialog(text: &str, title: &str, utype: u32) {
    extern "system" {
        fn MessageBoxW(hwnd: *mut std::ffi::c_void, text: *const u16, caption: *const u16, utype: u32) -> i32;
    }
    let t: Vec<u16> = text.encode_utf16().chain(std::iter::once(0)).collect();
    let c: Vec<u16> = title.encode_utf16().chain(std::iter::once(0)).collect();
    unsafe { MessageBoxW(std::ptr::null_mut(), t.as_ptr(), c.as_ptr(), utype); }
}

/// Muestra un InputBox de Windows usando VisualBasic (disponible en todas las versiones de Windows).
/// Devuelve None si el usuario cancela o no escribe nada.
#[cfg(windows)]
fn input_dialog(message: &str, title: &str) -> Option<String> {
    let script = format!(
        "Add-Type -AssemblyName Microsoft.VisualBasic; \
         [Microsoft.VisualBasic.Interaction]::InputBox('{}','{}','')",
        message.replace('\'', "''"),
        title.replace('\'', "''"),
    );
    #[allow(unused_imports)]
    use std::os::windows::process::CommandExt;
    let out = std::process::Command::new("powershell")
        .args(["-NoProfile", "-NonInteractive", "-Command", &script])
        .creation_flags(0x08000000) // CREATE_NO_WINDOW
        .output()
        .ok()?;
    let result = String::from_utf8_lossy(&out.stdout).trim().to_lowercase();
    if result.is_empty() { None } else { Some(result) }
}
