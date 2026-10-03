//! Runs only against an explicitly provided test database, in an isolated schema.
use crate::{auth, channel, db, kick, queue, server, test_support, webhook};
use axum::{
    body::Bytes,
    extract::State,
    http::{HeaderMap, StatusCode},
};
use base64::{engine::general_purpose::STANDARD, Engine};
use rsa::{
    pkcs1v15::SigningKey,
    signature::{SignatureEncoding, Signer},
    RsaPrivateKey, RsaPublicKey,
};
use serde_json::{json, Value};
use sha2::Sha256;
use std::{
    collections::HashMap,
    sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    },
};
use tokio::sync::Mutex;

#[tokio::test]
#[ignore = "requires TEST_DATABASE_URL"]
async fn oauth_pkce_browser_binding_reauthorization_and_failures() {
    use axum::{extract::Query, http::header};
    use sha2::Digest;
    let database = test_support::TestDatabase::new().await;
    let mock = Arc::new(MockKick::default());
    let service = mock.serve().await;
    let mut app = test_support::app(database.pool.clone());
    use_mock(&mut app, &service);

    let redirect = auth::redirect_to_kick(State(app.clone())).await;
    assert_eq!(redirect.status(), StatusCode::TEMPORARY_REDIRECT);
    let location = url::Url::parse(redirect.headers()[header::LOCATION].to_str().unwrap()).unwrap();
    let params: HashMap<String, String> = location.query_pairs().into_owned().collect();
    assert_eq!(params["code_challenge_method"], "S256");
    assert!(redirect.headers()[header::SET_COOKIE]
        .to_str()
        .unwrap()
        .contains("HttpOnly; SameSite=Lax"));
    let state = params["state"].clone();
    let query = HashMap::from([
        ("code".into(), "test-code".into()),
        ("state".into(), state.clone()),
    ]);
    // A callback from another browser must leave state available for its owner.
    let invalid =
        auth::handle_callback(State(app.clone()), HeaderMap::new(), Query(query.clone())).await;
    assert!(invalid.0.contains("sesion OAuth no coincide"));
    assert_eq!(mock.token_requests.load(Ordering::Relaxed), 0);
    let mut headers = HeaderMap::new();
    headers.insert(
        header::COOKIE,
        format!("daibot_oauth={state}").parse().unwrap(),
    );
    let (a, b) = tokio::join!(
        auth::handle_callback(State(app.clone()), headers.clone(), Query(query.clone())),
        auth::handle_callback(State(app.clone()), headers.clone(), Query(query.clone()))
    );
    assert_eq!(
        [a.0, b.0]
            .iter()
            .filter(|page| page.contains("¡DaiBot conectado"))
            .count(),
        1
    );
    assert_eq!(mock.token_requests.load(Ordering::Relaxed), 1);
    let forms = mock.forms.lock().await;
    assert_eq!(forms[0]["code"], "test-code");
    assert_eq!(
        forms[0]["redirect_uri"],
        "https://test.example/auth/callback"
    );
    assert_eq!(
        base64::engine::general_purpose::URL_SAFE_NO_PAD
            .encode(Sha256::digest(&forms[0]["code_verifier"])),
        params["code_challenge"]
    );
    drop(forms);

    let old = app.channels.get("alpha").unwrap().clone();
    let item = queue::VideoItem {
        id: "persistent".into(),
        video_id: None,
        url: Some("https://cdn.example/video.mp4".into()),
        title: "saved".into(),
        user: "viewer".into(),
    };
    queue::change(&old, &app, |q| {
        q.push(item);
        true
    })
    .await
    .unwrap();
    server::set_visibility(&old, &app, false).await.unwrap();
    let playback = old.playback_token.clone();
    db::save_oauth_state(&database.pool, &state, "next-verifier")
        .await
        .unwrap();
    let success =
        auth::handle_callback(State(app.clone()), headers.clone(), Query(query.clone())).await;
    assert!(success.0.contains(&playback));
    assert!(old.cancel.is_cancelled());
    let current = app.channels.get("alpha").unwrap().clone();
    assert!(!Arc::ptr_eq(&old, &current));
    assert_eq!(current.video_queue.read().await.items[0].id, "persistent");
    assert!(!current.show_video.load(Ordering::Relaxed));
    assert_eq!(current.playback_token, playback);
    assert_eq!(*current.access_token.read().await, "access-2");

    // Provider errors and invalid identity cannot replace an active channel.
    for (status, data, expected) in [
        (Some(StatusCode::BAD_REQUEST), None, "Kick rechazo"),
        (
            None,
            Some(json!({"data":[{"slug":"<script>","broadcaster_user_id":1}]})),
            "Identidad de canal invalida",
        ),
        (
            None,
            Some(json!({"data":[{"slug":"beta","broadcaster_user_id":0}]})),
            "Canal sin identificador valido",
        ),
    ] {
        *mock.token_status.lock().await = status;
        *mock.channel_data.lock().await = data;
        db::save_oauth_state(&database.pool, &state, "verifier")
            .await
            .unwrap();
        let result =
            auth::handle_callback(State(app.clone()), headers.clone(), Query(query.clone())).await;
        assert!(result.0.contains(expected), "{}", result.0);
        assert!(Arc::ptr_eq(&current, &app.channels.get("alpha").unwrap()));
        assert_eq!(
            db::load_all_channels(&database.pool).await.unwrap().len(),
            1
        );
    }
    app.shutdown.cancel();
    assert!(current.cancel.is_cancelled());
    database.cleanup().await;
}

#[tokio::test]
#[ignore = "requires TEST_DATABASE_URL"]
async fn refresh_serializes_rotation_honors_expiry_and_chat_retries_401() {
    let database = test_support::TestDatabase::new().await;
    db::upsert_channel(&database.pool, &test_support::row("alpha", 1))
        .await
        .unwrap();
    let mock = Arc::new(MockKick::default());
    let service = mock.serve().await;
    let mut app = test_support::app(database.pool.clone());
    use_mock(&mut app, &service);
    let (ch, _receiver) = test_support::channel("alpha", 1);
    let before = chrono::Utc::now().timestamp() as u64;
    let (a, b) = tokio::join!(
        kick::refresh_access_token(&ch, &app),
        kick::refresh_access_token(&ch, &app)
    );
    assert!(a && b);
    assert_eq!(mock.token_requests.load(Ordering::Relaxed), 1);
    let saved = db::load_channel(&database.pool, "alpha").await.unwrap();
    assert_eq!(saved.access_token, "access-1");
    assert_eq!(saved.refresh_token, "refresh-1");
    assert!((before + 90..=chrono::Utc::now().timestamp() as u64 + 90)
        .contains(&ch.token_expires.load(Ordering::Relaxed)));
    assert_eq!(
        saved.token_expires as u64,
        ch.token_expires.load(Ordering::Relaxed)
    );
    *mock.token_status.lock().await = Some(StatusCode::UNAUTHORIZED);
    assert!(!kick::refresh_access_token(&ch, &app).await);
    assert_eq!(*ch.access_token.read().await, "access-1");
    *mock.token_status.lock().await = None;
    *ch.access_token.write().await = "expired".into();
    kick::sender::send("hola á 😀", &ch, &app).await;
    let chats = mock.chat_requests.lock().await;
    assert_eq!(chats.len(), 2);
    assert_eq!(chats[0].0, "Bearer expired");
    assert_eq!(chats[1].0, "Bearer access-3");
    assert_eq!(chats[1].1["content"], "hola á 😀");
    drop(chats);
    ch.cancel.cancel();
    assert!(!kick::refresh_access_token(&ch, &app).await);
    assert_eq!(mock.token_requests.load(Ordering::Relaxed), 3);
    database.cleanup().await;
}

#[tokio::test]
#[ignore = "requires TEST_DATABASE_URL"]
async fn queue_concurrency_conflicts_db_failures_and_readiness() {
    let database = test_support::TestDatabase::new().await;
    db::upsert_channel(&database.pool, &test_support::row("alpha", 1))
        .await
        .unwrap();
    let app = test_support::app(database.pool.clone());
    let (ch, _receiver) = test_support::channel("alpha", 1);
    let item = queue::VideoItem {
        id: "one".into(),
        video_id: None,
        url: Some("https://cdn.example/video.mp4".into()),
        title: "one".into(),
        user: "viewer".into(),
    };
    queue::change(&ch, &app, |q| {
        q.push(item.clone());
        q.push(queue::VideoItem {
            id: "two".into(),
            ..item
        });
        true
    })
    .await
    .unwrap();
    let (a, b) = tokio::join!(
        queue::change(&ch, &app, |q| q.advance_if("one", 2)),
        queue::change(&ch, &app, |q| q.advance_if("one", 2))
    );
    assert_eq!(
        [a.unwrap(), b.unwrap()]
            .iter()
            .filter(|result| **result)
            .count(),
        1
    );
    // A stale state must reload a competing writer's queue instead of overwriting it.
    let (stale, _rx) = test_support::channel("alpha", 1);
    assert!(queue::change(&stale, &app, |q| {
        q.clear();
        true
    })
    .await
    .is_err());
    assert_eq!(stale.video_queue.read().await.items[0].id, "two");
    assert_eq!(stale.video_queue.read().await.version, 3);
    assert_eq!(crate::ready(State(app.clone())).await, StatusCode::OK);
    database.pool.close().await;
    assert!(queue::change(&ch, &app, |q| {
        q.clear();
        true
    })
    .await
    .is_err());
    assert_eq!(ch.video_queue.read().await.items[0].id, "two");
    assert!(server::set_visibility(&ch, &app, false).await.is_err());
    assert!(ch.show_video.load(Ordering::Relaxed));
    assert_eq!(
        crate::ready(State(app.clone())).await,
        StatusCode::SERVICE_UNAVAILABLE
    );
    assert_eq!(
        auth::redirect_to_kick(State(app.clone())).await.status(),
        StatusCode::SERVICE_UNAVAILABLE
    );
    app.shutdown.cancel();
    database.cleanup().await;
}

#[derive(Default)]
struct MockKick {
    forms: Mutex<Vec<HashMap<String, String>>>,
    token_requests: AtomicUsize,
    token_status: Mutex<Option<StatusCode>>,
    channel_data: Mutex<Option<Value>>,
    chat_requests: Mutex<Vec<(String, Value)>>,
}

impl MockKick {
    async fn serve(self: &Arc<Self>) -> test_support::HttpServer {
        async fn token(
            State(mock): State<Arc<MockKick>>,
            axum::Form(form): axum::Form<HashMap<String, String>>,
        ) -> (StatusCode, axum::Json<Value>) {
            mock.forms.lock().await.push(form);
            let n = mock.token_requests.fetch_add(1, Ordering::Relaxed) + 1;
            // Keep concurrent refresh callers in flight until they all observe the old token.
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
            let status = mock.token_status.lock().await.unwrap_or(StatusCode::OK);
            (
                status,
                axum::Json(
                    json!({"access_token":format!("access-{n}"), "refresh_token":format!("refresh-{n}"), "expires_in":90}),
                ),
            )
        }
        async fn channels(State(mock): State<Arc<MockKick>>) -> axum::Json<Value> {
            axum::Json(mock.channel_data.lock().await.clone().unwrap_or_else(|| json!({"data":[{"slug":"alpha","broadcaster_user_id":1,"stream":{"is_live":false}}]})))
        }
        async fn chat(
            State(mock): State<Arc<MockKick>>,
            headers: HeaderMap,
            axum::Json(body): axum::Json<Value>,
        ) -> StatusCode {
            let token = headers
                .get("authorization")
                .unwrap()
                .to_str()
                .unwrap()
                .to_owned();
            mock.chat_requests.lock().await.push((token.clone(), body));
            if token == "Bearer expired" {
                StatusCode::UNAUTHORIZED
            } else {
                StatusCode::OK
            }
        }
        test_support::HttpServer::start(
            axum::Router::new()
                .route("/oauth/token", axum::routing::post(token))
                .route("/public/v1/channels", axum::routing::get(channels))
                .route("/public/v1/chat", axum::routing::post(chat))
                .route(
                    "/public/v1/events/subscriptions",
                    axum::routing::post(|| async { StatusCode::OK }),
                )
                .with_state(self.clone()),
        )
        .await
    }
}

fn use_mock(app: &mut Arc<crate::state::AppState>, mock: &test_support::HttpServer) {
    Arc::get_mut(app).unwrap().kick_endpoints = kick::Endpoints {
        oauth: format!("{}/oauth", mock.url),
        api: format!("{}/public/v1", mock.url),
    };
}

#[tokio::test]
#[ignore = "requires TEST_DATABASE_URL; executed by CI with PostgreSQL"]
async fn database_persistence_oauth_and_webhook_idempotency() {
    let url = std::env::var("TEST_DATABASE_URL").expect("use an isolated test PostgreSQL database");
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
    db::run_migrations(&pool).await.unwrap();
    db::run_migrations(&pool).await.unwrap();
    let mut app = test_support::app(pool.clone());
    let private = RsaPrivateKey::new(&mut rand::thread_rng(), 2048).unwrap();
    std::sync::Arc::get_mut(&mut app).unwrap().webhook_key = RsaPublicKey::from(&private);
    let mut receivers = HashMap::new();
    for (slug, id) in [("alpha", 1), ("beta", 2)] {
        let row = db::ChannelRow {
            slug: slug.into(),
            broadcaster_user_id: Some(id),
            chatroom_id: Some(id),
            access_token: "access".into(),
            refresh_token: "refresh".into(),
            token_expires: 0,
            panel_token: "panel-secret".into(),
            cmd_discord: "saved-setting".into(),
            cmd_redes: String::new(),
            cmd_pc: String::new(),
            cmd_horario: String::new(),
            follow_goal: 200,
            queue_state: "{\"items\":[],\"version\":0}".into(),
            show_video: true,
            playback_token: "play-secret".into(),
        };
        db::upsert_channel(&pool, &row).await.unwrap();
        let (ch, rx) = test_support::channel(slug, id as u64);
        receivers.insert(slug, rx);
        app.channels.insert(slug.into(), ch);
        app.user_id_to_slug.insert(id as u64, slug.into());
    }
    let alpha = app.channels.get("alpha").unwrap().clone();
    let beta = app.channels.get("beta").unwrap().clone();
    let first = queue::VideoItem {
        id: "unique-one".into(),
        video_id: Some("dQw4w9WgXcQ".into()),
        url: None,
        title: "same video".into(),
        user: "viewer".into(),
    };
    queue::change(&alpha, &app, |q| {
        q.push(first.clone());
        q.push(queue::VideoItem {
            id: "unique-two".into(),
            ..first
        });
        true
    })
    .await
    .unwrap();
    assert!(beta.video_queue.read().await.items.is_empty());
    let loaded = db::load_channel(&pool, "alpha").await.unwrap();
    let restored: queue::VideoQueue = serde_json::from_str(&loaded.queue_state).unwrap();
    assert_eq!(restored.items.len(), 2);
    assert_eq!(restored.version, 2);
    assert!(
        queue::change(&alpha, &app, |q| q.advance_if("unique-one", 2))
            .await
            .unwrap()
    );
    assert!(
        !queue::change(&alpha, &app, |q| q.advance_if("unique-one", 2))
            .await
            .unwrap()
    );
    // Reauthorizing must preserve user settings, playback token and persisted queue.
    let mut login = loaded;
    login.cmd_discord.clear();
    login.follow_goal = 100;
    login.playback_token = "replacement".into();
    db::upsert_channel(&pool, &login).await.unwrap();
    let reloaded = db::load_channel(&pool, "alpha").await.unwrap();
    assert_eq!(reloaded.cmd_discord, "saved-setting");
    assert_eq!(reloaded.follow_goal, 200);
    assert_eq!(reloaded.playback_token, "play-secret");
    assert_eq!(
        serde_json::from_str::<queue::VideoQueue>(&reloaded.queue_state)
            .unwrap()
            .items
            .len(),
        1
    );

    db::save_oauth_state(&pool, "single-use", "verifier")
        .await
        .unwrap();
    let (a, b) = tokio::join!(
        db::consume_oauth_state(&pool, "single-use"),
        db::consume_oauth_state(&pool, "single-use")
    );
    assert_eq!(
        [a.unwrap(), b.unwrap()]
            .iter()
            .filter(|v| v.is_some())
            .count(),
        1
    );
    db::save_oauth_state(&pool, "expired", "verifier")
        .await
        .unwrap();
    sqlx::query("UPDATE oauth_state SET created_at=$1 WHERE token='expired'")
        .bind(chrono::Utc::now().timestamp() - 601)
        .execute(&pool)
        .await
        .unwrap();
    assert!(db::consume_oauth_state(&pool, "expired")
        .await
        .unwrap()
        .is_none());

    let timestamp = chrono::Utc::now().to_rfc3339();
    let body = br#"{"broadcaster":{"user_id":1},"follower":{"username":"viewer"}}"#;
    let mut message = format!("unique-event.{timestamp}.").into_bytes();
    message.extend_from_slice(body);
    let signature = SigningKey::<Sha256>::new(private).sign(&message);
    let mut headers = HeaderMap::new();
    headers.insert("Kick-Event-Message-Id", "unique-event".parse().unwrap());
    headers.insert("Kick-Event-Message-Timestamp", timestamp.parse().unwrap());
    headers.insert("Kick-Event-Type", "channel.followed".parse().unwrap());
    headers.insert(
        "Kick-Event-Signature",
        STANDARD.encode(signature.to_bytes()).parse().unwrap(),
    );
    for _ in 0..2 {
        assert_eq!(
            webhook::receive(
                State(app.clone()),
                headers.clone(),
                Bytes::from_static(body)
            )
            .await,
            StatusCode::OK
        );
    }
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM webhook_events")
            .fetch_one(&pool)
            .await
            .unwrap(),
        1
    );
    assert_eq!(
        webhook::receive(
            State(app.clone()),
            headers.clone(),
            Bytes::from_static(b"altered")
        )
        .await,
        StatusCode::UNAUTHORIZED
    );
    let worker = tokio::spawn(webhook::worker(app.clone()));
    tokio::time::timeout(std::time::Duration::from_secs(5), async {
        loop {
            if sqlx::query_scalar::<_, bool>("SELECT processed FROM webhook_events")
                .fetch_one(&pool)
                .await
                .unwrap()
            {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
    })
    .await
    .unwrap();
    app.shutdown.cancel();
    worker.await.unwrap();
    assert!(receivers
        .get_mut("alpha")
        .unwrap()
        .try_recv()
        .unwrap()
        .text
        .contains("viewer"));
    assert!(receivers.get_mut("alpha").unwrap().try_recv().is_err());
    assert!(receivers.get_mut("beta").unwrap().try_recv().is_err());
    assert_eq!(app.metrics.webhook_processed.load(Ordering::Relaxed), 1);
    assert_eq!(app.metrics.webhook_duplicates.load(Ordering::Relaxed), 1);
    pool.close().await;
    assert!(db::upsert_channel(&pool, &login).await.is_err());
    sqlx::query(&format!("DROP SCHEMA {schema} CASCADE"))
        .execute(&admin)
        .await
        .unwrap();
    admin.close().await;
}
