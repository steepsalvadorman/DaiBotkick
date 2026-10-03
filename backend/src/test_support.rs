use crate::state::{AppState, ChannelCommands, ChannelState, GlobalConfig, SorteoState};
use dashmap::DashMap;
use std::sync::{
    atomic::{AtomicBool, AtomicU64},
    Arc,
};
use tokio::sync::{mpsc, Mutex, RwLock, Semaphore};
use tokio_util::sync::CancellationToken;

pub fn app(db: sqlx::PgPool) -> Arc<AppState> {
    let (_, io) = socketioxide::SocketIo::new_layer();
    app_with_io(db, io)
}

pub fn app_with_io(db: sqlx::PgPool, io: socketioxide::SocketIo) -> Arc<AppState> {
    io.ns("/", |_: socketioxide::extract::SocketRef| {});
    Arc::new(AppState {
        config: GlobalConfig {
            client_id: "test".into(),
            client_secret: "test".into(),
            port: 3000,
            overlay_dir: "../overlay".into(),
            base_url: "https://test.example".into(),
            tts_cache_dir: std::env::temp_dir()
                .join("daibot_test")
                .to_string_lossy()
                .into_owned(),
        },
        http: reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(5))
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .unwrap(),
        kick_endpoints: crate::kick::Endpoints::default(),
        io,
        db,
        channels: Arc::new(DashMap::new()),
        user_id_to_slug: Arc::new(DashMap::new()),
        channel_lock: Mutex::new(()),
        shutdown: CancellationToken::new(),
        tts_slots: Arc::new(Semaphore::new(2)),
        webhook_key: crate::webhook::public_key(),
        metrics: Arc::new(crate::state::Metrics::default()),
    })
}

pub struct TestDatabase {
    pub pool: sqlx::PgPool,
    admin: sqlx::PgPool,
    schema: String,
}

impl TestDatabase {
    pub async fn new() -> Self {
        let url = std::env::var("TEST_DATABASE_URL")
            .expect("TEST_DATABASE_URL must point to an isolated test database");
        let admin = sqlx::PgPool::connect(&url).await.unwrap();
        let schema = format!("daibot_test_{}", uuid::Uuid::new_v4().simple());
        sqlx::query(&format!("CREATE SCHEMA {schema}"))
            .execute(&admin)
            .await
            .unwrap();
        let options = url
            .parse::<sqlx::postgres::PgConnectOptions>()
            .unwrap()
            .options([("search_path", schema.as_str())]);
        let pool = sqlx::postgres::PgPoolOptions::new()
            .max_connections(5)
            .connect_with(options)
            .await
            .unwrap();
        crate::db::run_migrations(&pool).await.unwrap();
        Self {
            pool,
            admin,
            schema,
        }
    }

    pub async fn cleanup(self) {
        self.pool.close().await;
        sqlx::query(&format!("DROP SCHEMA {} CASCADE", self.schema))
            .execute(&self.admin)
            .await
            .unwrap();
        self.admin.close().await;
    }
}

pub fn row(slug: &str, id: i64) -> crate::db::ChannelRow {
    crate::db::ChannelRow {
        slug: slug.into(),
        broadcaster_user_id: Some(id),
        chatroom_id: Some(id),
        access_token: "access".into(),
        refresh_token: "refresh".into(),
        token_expires: chrono::Utc::now().timestamp() + 3600,
        panel_token: "panel-secret".into(),
        cmd_discord: "saved-setting".into(),
        cmd_redes: String::new(),
        cmd_pc: String::new(),
        cmd_horario: String::new(),
        follow_goal: 200,
        queue_state: "{\"items\":[],\"version\":0}".into(),
        show_video: true,
        playback_token: "play-secret".into(),
    }
}

pub struct HttpServer {
    pub url: String,
    task: tokio::task::JoinHandle<()>,
}

impl HttpServer {
    pub async fn start(router: axum::Router) -> Self {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let task = tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
        Self { url, task }
    }
}

impl Drop for HttpServer {
    fn drop(&mut self) {
        self.task.abort();
    }
}
pub fn channel(
    slug: &str,
    id: u64,
) -> (Arc<ChannelState>, mpsc::Receiver<crate::tts::TtsQueueItem>) {
    let (tts_tx, rx) = mpsc::channel(32);
    (
        Arc::new(ChannelState {
            slug: slug.into(),
            access_token: Arc::new(RwLock::new("access".into())),
            refresh_token_val: Arc::new(RwLock::new("refresh".into())),
            persisted_refresh_token: RwLock::new("refresh".into()),
            channel_id: Arc::new(RwLock::new(Some(id))),
            followers: Arc::new(AtomicU64::new(0)),
            followers_known: AtomicBool::new(false),
            follow_goal: 100,
            video_queue: Arc::new(RwLock::new(crate::queue::VideoQueue::new())),
            tts_tx,
            sorteo: Arc::new(Mutex::new(SorteoState {
                open: false,
                participants: Vec::new(),
            })),
            cooldown: Arc::new(Mutex::new(crate::cooldown::CooldownManager::new())),
            commands: ChannelCommands {
                discord: String::new(),
                redes: String::new(),
                pc: String::new(),
                horario: String::new(),
            },
            panel_token: "panel-secret".into(),
            playback_token: "play-secret".into(),
            cancel: CancellationToken::new(),
            refresh_lock: Mutex::new(()),
            token_expires: AtomicU64::new(0),
            player: Mutex::new(None),
            show_video: AtomicBool::new(true),
            live_since: RwLock::new(None),
        }),
        rx,
    )
}
