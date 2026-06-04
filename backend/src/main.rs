#![cfg_attr(windows, windows_subsystem = "windows")]

mod commands;
mod config;
mod cooldown;
mod kick;
mod login;
mod queue;
mod server;
mod stats;
mod tts;

use config::Config;
use queue::VideoQueue;
use socketioxide::SocketIo;
use std::sync::{
    atomic::AtomicU64,
    Arc,
};
use tokio::sync::{mpsc, Mutex, RwLock};
use tower_http::services::ServeDir;
use tracing::info;
use tracing_subscriber::EnvFilter;

pub struct SorteoState {
    pub open:         bool,
    pub participants: Vec<String>,
}

pub struct AppState {
    pub config:       Config,
    pub video_queue:  Arc<RwLock<VideoQueue>>,
    pub io:           SocketIo,
    pub tts_tx:       mpsc::UnboundedSender<tts::TtsQueueItem>,
    pub http:         reqwest::Client,
    /// channel_id del canal (broadcaster_user_id para la API oficial de chat)
    pub channel_id:   Arc<RwLock<Option<u64>>>,
    /// chatroom_id del canal (para la API legacy y Pusher)
    pub chatroom_id:  Arc<RwLock<Option<u64>>>,
    /// Contador de seguidores actualizado por Pusher en tiempo real
    pub followers:    Arc<AtomicU64>,
    /// Token OAuth activo (se puede renovar sin reiniciar)
    pub access_token:      Arc<RwLock<String>>,
    pub refresh_token_val: Arc<RwLock<String>>,
    /// Evita doble-avance cuando varios overlays envían advanceQueue a la vez
    pub last_advance: Arc<Mutex<Option<std::time::Instant>>>,
    /// Momento en que arrancó el bot (para !uptime)
    pub start_time:   std::time::Instant,
    /// Estado del sorteo activo
    pub sorteo:       Arc<Mutex<SorteoState>>,
    /// Anti-spam: cooldowns por usuario y globales
    pub cooldown:     Arc<Mutex<cooldown::CooldownManager>>,
}

fn resolve_overlay_dir(configured: &str) -> std::path::PathBuf {
    let p = std::path::Path::new(configured);
    if p.is_absolute() && p.exists() {
        return p.to_path_buf();
    }

    // Intentar relativo al exe
    if let Ok(exe) = std::env::current_exe() {
        let candidate = exe.parent().map(|d| d.join(configured)).unwrap_or_default();
        if candidate.exists() {
            return candidate;
        }
        // Subir por los ancestros del exe buscando la carpeta overlay
        for ancestor in exe.ancestors().skip(1) {
            let candidate = ancestor.join("overlay");
            if candidate.join("pixel.html").exists() {
                return candidate;
            }
        }
    }

    // Fallback: working dir
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
    #[cfg(windows)]
    {
        extern "system" {
            fn MessageBoxW(hwnd: *mut std::ffi::c_void, text: *const u16, caption: *const u16, utype: u32) -> i32;
        }
        let text: Vec<u16> = msg.encode_utf16().chain(std::iter::once(0)).collect();
        let cap: Vec<u16>  = "DaiBot".encode_utf16().chain(std::iter::once(0)).collect();
        unsafe { MessageBoxW(std::ptr::null_mut(), text.as_ptr(), cap.as_ptr(), 0x10); }
    }
    #[cfg(not(windows))]
    {
        eprintln!("\n{msg}\n");
        eprintln!("Presiona Enter para cerrar...");
        let mut buf = String::new();
        let _ = std::io::stdin().read_line(&mut buf);
    }
    std::process::exit(1);
}

#[tokio::main]
async fn main() {
    if std::env::args().any(|a| a == "--login") {
        // Abrir consola visible para el flujo OAuth (sin ella no hay stdin/stdout)
        #[cfg(windows)]
        unsafe {
            extern "system" { fn AllocConsole() -> i32; }
            AllocConsole();
        }
        login::run_and_exit().await;
        return;
    }

    load_dotenv();

    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .init();

    // Si no hay credenciales es el primer arranque → login automático
    if let Err(_) = Config::from_env() {
        login::run_first_time().await;
        load_dotenv(); // recargar .env con los tokens recién guardados
    }

    let config = Config::from_env().unwrap_or_else(|e| fatal(&e));

    // Limpiar la cola al arrancar para evitar videos stale de sesiones anteriores
    let _ = std::fs::write(&config.queue_file, "[]");
    let video_queue = Arc::new(RwLock::new(VideoQueue::load(&config.queue_file)));
    let (layer, io) = SocketIo::new_layer();

    let tts_svc = Arc::new(tts::TtsService::new(&config.tts_cache_dir));
    let (tts_tx, tts_rx) = mpsc::unbounded_channel::<tts::TtsQueueItem>();
    tts::spawn_processor(tts_svc, tts_rx, io.clone());

    let state = Arc::new(AppState {
        access_token:      Arc::new(RwLock::new(config.access_token.clone())),
        refresh_token_val: Arc::new(RwLock::new(config.refresh_token.clone())),
        config: config.clone(),
        video_queue,
        io: io.clone(),
        tts_tx,
        http: reqwest::Client::builder()
            .user_agent("Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/124.0.0.0 Safari/537.36")
            .build()
            .unwrap(),
        channel_id:   Arc::new(RwLock::new(None)),
        chatroom_id:  Arc::new(RwLock::new(None)),
        followers:    Arc::new(AtomicU64::new(config.current_followers)),
        last_advance: Arc::new(Mutex::new(None)),
        start_time:   std::time::Instant::now(),
        sorteo:       Arc::new(Mutex::new(SorteoState { open: false, participants: Vec::new() })),
        cooldown:     Arc::new(Mutex::new(cooldown::CooldownManager::new())),
    });

    // Renovar token automáticamente antes de que expire
    tokio::spawn(kick::token_refresh_loop(state.clone(), config.token_expires));

    server::setup(&io, state.clone());
    stats::start(io.clone(), state.clone());
    tokio::spawn(kick::run(state.clone()));

    // Resolver overlay_dir: primero relativo al exe, luego buscar subiendo si no existe
    let overlay_dir = resolve_overlay_dir(&config.overlay_dir);
    info!("Sirviendo overlay desde: {}", overlay_dir.display());

    let app = axum::Router::new()
        .nest_service("/", ServeDir::new(&overlay_dir))
        .layer(layer);

    let addr = format!("0.0.0.0:{}", config.port);
    let listener = tokio::net::TcpListener::bind(&addr).await.unwrap();
    info!("DaiBot corriendo  → http://localhost:{}", config.port);
    info!("Panel de control  → http://localhost:{}/panel.html", config.port);

    // Popup gaming (WPF nativo, en thread aparte para no bloquear el servidor)
    #[cfg(windows)]
    std::thread::spawn(|| {
        let script = r##"
Add-Type -AssemblyName PresentationFramework
Add-Type -AssemblyName PresentationCore
Add-Type -AssemblyName WindowsBase

[xml]$xaml = @"
<Window
    xmlns="http://schemas.microsoft.com/winfx/2006/xaml/presentation"
    xmlns:x="http://schemas.microsoft.com/winfx/2006/xaml"
    Title="DaiBot" Width="480" Height="260"
    WindowStyle="None" AllowsTransparency="True"
    Background="Transparent" WindowStartupLocation="CenterScreen"
    Topmost="True" ResizeMode="NoResize">
  <Border Background="#060010" BorderBrush="#53FC18" BorderThickness="2" Padding="40,28">
    <Border.Effect>
      <DropShadowEffect Color="#53FC18" BlurRadius="32" ShadowDepth="0" Opacity="0.55"/>
    </Border.Effect>
    <StackPanel>
      <TextBlock Text="[ D A I B O T ]"
                 Foreground="#53FC18" FontSize="22" FontWeight="Black"
                 HorizontalAlignment="Center" FontFamily="Consolas">
        <TextBlock.Effect>
          <DropShadowEffect Color="#53FC18" BlurRadius="14" ShadowDepth="0" Opacity="0.9"/>
        </TextBlock.Effect>
      </TextBlock>
      <Rectangle Height="1" Fill="#53FC18" Margin="0,16,0,8"/>
      <TextBlock Text="&gt;&gt; STATUS: ONLINE" Foreground="#53FC18" FontSize="10"
                 FontFamily="Consolas" HorizontalAlignment="Center" Opacity="0.75"/>
      <Rectangle Height="1" Fill="#53FC18" Margin="0,8,0,16" Opacity="0.4"/>
      <TextBlock Text="Todo listo para Karkagarla en lefordi"
                 Foreground="#DDDDDD" FontSize="12" HorizontalAlignment="Center"
                 FontFamily="Consolas" TextWrapping="Wrap" TextAlignment="Center"
                 MaxWidth="370" LineHeight="20"/>
      <Rectangle Height="1" Fill="#53FC18" Margin="0,18,0,16" Opacity="0.3"/>
      <Button x:Name="OkBtn" Content="[  OK  ]" HorizontalAlignment="Center"
              Background="#53FC18" Foreground="#060010" FontWeight="Black" FontSize="11"
              FontFamily="Consolas" Padding="28,8" BorderThickness="0" Cursor="Hand"/>
    </StackPanel>
  </Border>
</Window>
"@

$reader = [System.Xml.XmlReader]::Create([System.IO.StringReader]::new($xaml.OuterXml))
$window = [System.Windows.Markup.XamlReader]::Load($reader)

$window.FindName('OkBtn').Add_Click({ $window.Close() })

# Cerrar automaticamente en 8 segundos
$timer = New-Object System.Windows.Threading.DispatcherTimer
$timer.Interval = [System.TimeSpan]::FromSeconds(8)
$timer.Add_Tick({ $window.Close(); $timer.Stop() })
$timer.Start()

$null = $window.ShowDialog()
"##;
        let tmp = std::env::temp_dir().join("daibot_notify.ps1");
        // BOM UTF-8 obligatorio para PowerShell 5.1 (sin él lee como Windows-1252)
        let mut content = vec![0xEF_u8, 0xBB, 0xBF];
        content.extend_from_slice(script.as_bytes());
        let _ = std::fs::write(&tmp, &content);
        use std::os::windows::process::CommandExt;
        let _ = std::process::Command::new("powershell")
            .args(["-NoProfile", "-ExecutionPolicy", "Bypass", "-File",
                   tmp.to_str().unwrap_or("")])
            .creation_flags(0x08000000)
            .spawn()
            .and_then(|mut c| c.wait());
        let _ = std::fs::remove_file(&tmp);
    });

    axum::serve(listener, app).await.unwrap();
}
