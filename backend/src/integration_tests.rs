//! Runs only against an explicitly provided test database, in an isolated schema.
use crate::{auth, channel, db, kick, platform::Platform, queue, server, test_support, webhook};
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
    let saved = db::load_channel(&database.pool, Platform::Kick, "alpha")
        .await
        .unwrap();
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
    assert_eq!(chats[1].1["type"], "bot");
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
    block_chat: std::sync::atomic::AtomicBool,
    release_chat: tokio::sync::Notify,
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
            let n = {
                let mut requests = mock.chat_requests.lock().await;
                requests.push((token.clone(), body));
                requests.len()
            };
            if mock.block_chat.load(Ordering::Relaxed) {
                mock.release_chat.notified().await;
            }
            if token == "Bearer expired" {
                tokio::time::sleep(std::time::Duration::from_millis(if n == 1 {
                    5
                } else {
                    150
                }))
                .await;
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

#[tokio::test]
#[ignore = "requires TEST_DATABASE_URL"]
async fn durable_inbox_retries_after_interrupted_external_effect() {
    let database = test_support::TestDatabase::new().await;
    db::upsert_channel(&database.pool, &test_support::row("alpha", 1))
        .await
        .unwrap();
    let mock = Arc::new(MockKick::default());
    mock.block_chat.store(true, Ordering::Relaxed);
    let service = mock.serve().await;
    let mut app = test_support::app(database.pool.clone());
    use_mock(&mut app, &service);
    let (ch, _receiver) = test_support::channel("alpha", 1);
    app.channels.insert("alpha".into(), ch);
    app.user_id_to_key
        .insert((Platform::Kick, 1), "alpha".into());
    let payload = json!({"broadcaster":{"user_id":1},"sender":{"user_id":1,"username":"alpha"},"content":"!comandos"});
    sqlx::query("INSERT INTO webhook_events (message_id,received_at,payload,event_type) VALUES ('interrupted',$1,$2,'chat.message.sent')")
        .bind(chrono::Utc::now().timestamp()).bind(payload.to_string()).execute(&database.pool).await.unwrap();
    let worker = tokio::spawn(webhook::worker(app.clone()));
    tokio::time::timeout(std::time::Duration::from_secs(3), async {
        while mock.chat_requests.lock().await.is_empty() {
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    app.shutdown.cancel();
    worker.await.unwrap();
    assert!(
        !sqlx::query_scalar::<_, bool>("SELECT processed FROM webhook_events")
            .fetch_one(&database.pool)
            .await
            .unwrap()
    );
    mock.block_chat.store(false, Ordering::Relaxed);
    mock.release_chat.notify_waiters();
    let mut restarted = test_support::app(database.pool.clone());
    use_mock(&mut restarted, &service);
    let (ch, _receiver) = test_support::channel("alpha", 1);
    restarted.channels.insert("alpha".into(), ch);
    restarted
        .user_id_to_key
        .insert((Platform::Kick, 1), "alpha".into());
    let worker = tokio::spawn(webhook::worker(restarted.clone()));
    tokio::time::timeout(std::time::Duration::from_secs(3), async {
        while !sqlx::query_scalar::<_, bool>("SELECT processed FROM webhook_events")
            .fetch_one(&database.pool)
            .await
            .unwrap()
        {
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    // The first external send may already have happened: recovery permits a duplicate effect.
    assert_eq!(mock.chat_requests.lock().await.len(), 2);
    assert_eq!(
        restarted.metrics.webhook_processed.load(Ordering::Relaxed),
        1
    );
    restarted.shutdown.cancel();
    worker.await.unwrap();
    database.cleanup().await;
}

#[tokio::test]
#[ignore = "requires TEST_DATABASE_URL"]
async fn concurrent_late_401_responses_share_one_refresh() {
    let database = test_support::TestDatabase::new().await;
    db::upsert_channel(&database.pool, &test_support::row("alpha", 1))
        .await
        .unwrap();
    let mock = Arc::new(MockKick::default());
    let service = mock.serve().await;
    let mut app = test_support::app(database.pool.clone());
    use_mock(&mut app, &service);
    let (ch, _receiver) = test_support::channel("alpha", 1);
    *ch.access_token.write().await = "expired".into();
    tokio::join!(
        kick::sender::send("first", &ch, &app),
        kick::sender::send("second", &ch, &app)
    );
    assert_eq!(mock.token_requests.load(Ordering::Relaxed), 1);
    let requests = mock.chat_requests.lock().await;
    assert_eq!(requests.len(), 4);
    assert_eq!(
        requests
            .iter()
            .filter(|(token, _)| token == "Bearer access-1")
            .count(),
        2
    );
    drop(requests);
    database.cleanup().await;
}

fn use_mock(app: &mut Arc<crate::state::AppState>, mock: &test_support::HttpServer) {
    Arc::get_mut(app).unwrap().kick_endpoints = kick::Endpoints {
        oauth: format!("{}/oauth", mock.url),
        api: format!("{}/public/v1", mock.url),
    };
}

/// Uses the real Engine.IO polling transport, without a JavaScript mock.
struct SocketClient {
    http: reqwest::Client,
    url: String,
    events: std::collections::VecDeque<Value>,
}

impl SocketClient {
    async fn connect(base: &str, slug: &str, token: Option<&str>) -> Self {
        let http = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(3))
            .build()
            .unwrap();
        let url = format!("{base}/socket.io/?EIO=4&transport=polling&ch={slug}");
        let handshake = http
            .get(&url)
            .send()
            .await
            .unwrap()
            .error_for_status()
            .unwrap()
            .text()
            .await
            .unwrap();
        let open: Value = serde_json::from_str(handshake.strip_prefix('0').unwrap()).unwrap();
        let url = format!("{url}&sid={}", open["sid"].as_str().unwrap());
        let socket = Self {
            http,
            url,
            events: Default::default(),
        };
        socket.packet(format!("40{}", json!({"token":token}))).await;
        socket
    }

    async fn packet(&self, packet: String) {
        self.http
            .post(&self.url)
            .header("content-type", "text/plain")
            .body(packet)
            .send()
            .await
            .unwrap()
            .error_for_status()
            .unwrap();
    }

    async fn emit(&self, name: &str, data: Value) {
        self.packet(format!("42{}", json!([name, data]))).await;
    }

    async fn event(&mut self, name: &str) -> Value {
        tokio::time::timeout(std::time::Duration::from_secs(3), async {
            loop {
                if let Some(index) = self.events.iter().position(|event| event[0] == name) {
                    return self.events.remove(index).unwrap()[1].clone();
                }
                let body = self
                    .http
                    .get(&self.url)
                    .send()
                    .await
                    .unwrap()
                    .error_for_status()
                    .unwrap()
                    .text()
                    .await
                    .unwrap();
                for packet in body.split('\u{1e}') {
                    if let Some(event) = packet.strip_prefix("42") {
                        self.events.push_back(serde_json::from_str(event).unwrap());
                    } else if packet == "2" {
                        self.packet("3".into()).await;
                    }
                }
            }
        })
        .await
        .unwrap_or_else(|_| panic!("Socket.IO event {name} did not arrive"))
    }
}

#[tokio::test]
async fn tts_audio_reaches_the_player_socket_and_skips_public_overlays() {
    use sha2::Digest;
    let pool = sqlx::postgres::PgPoolOptions::new()
        .connect_lazy("postgres://unused:unused@127.0.0.1/unused")
        .unwrap();
    let (layer, io) = socketioxide::SocketIo::new_layer();
    let app = test_support::app_with_io(pool, io);
    let (ch, receiver) = test_support::channel("alpha", 1);
    app.channels.insert("alpha".into(), ch.clone());
    server::setup(&app.io, app.clone());
    let http = test_support::HttpServer::start(axum::Router::new().layer(layer)).await;
    let mut public = SocketClient::connect(&http.url, "alpha", None).await;
    let mut player = SocketClient::connect(&http.url, "alpha", Some("play-secret")).await;
    assert_eq!(public.event("playbackRole").await["active"], false);
    assert_eq!(player.event("playbackRole").await["active"], true);

    // Audio ya en caché: el procesador lo envía sin sintetizar.
    let cache = std::env::temp_dir().join(format!(
        "daibot_tts_test_{}_{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&cache).unwrap();
    for (text, mp3) in [("Hola", "ID3-hola"), ("Adios", "ID3-adios")] {
        let hash = Sha256::digest(format!("camila:{text}"));
        std::fs::write(cache.join(format!("{hash:x}.mp3")), mp3).unwrap();
    }
    let service = Arc::new(crate::tts::TtsService::new(cache.to_str().unwrap(), ""));
    tokio::spawn(crate::tts::spawn_processor(
        service,
        receiver,
        app.io.clone(),
        ch.clone(),
        app.tts_slots.clone(),
    ));
    let decode = |event: Value| {
        STANDARD
            .decode(event["audioBase64"].as_str().unwrap())
            .unwrap()
    };

    // Audio emitido antes de que el overlay pida datos: queda en el buffer del polling.
    assert!(crate::tts::enqueue(&ch, "Hola", "camila"));
    tokio::time::sleep(std::time::Duration::from_millis(200)).await;
    assert_eq!(decode(player.event("speak").await), b"ID3-hola");

    // Audio emitido con una petición de long-polling ya abierta.
    let late = ch.clone();
    tokio::spawn(async move {
        tokio::time::sleep(std::time::Duration::from_millis(200)).await;
        assert!(crate::tts::enqueue(&late, "Adios", "camila"));
    });
    assert_eq!(decode(player.event("speak").await), b"ID3-adios");

    let public_body = tokio::time::timeout(std::time::Duration::from_millis(500), async {
        let body = public.http.get(&public.url).send().await.unwrap();
        body.text().await.unwrap()
    })
    .await
    .unwrap_or_default();
    assert!(!public_body.contains("speak"));
    let _ = std::fs::remove_dir_all(cache);
}

#[tokio::test]
async fn player_ping_confirms_only_the_current_epoch() {
    let pool = sqlx::postgres::PgPoolOptions::new()
        .connect_lazy("postgres://unused:unused@127.0.0.1/unused")
        .unwrap();
    let (layer, io) = socketioxide::SocketIo::new_layer();
    let app = test_support::app_with_io(pool, io);
    let (ch, _receiver) = test_support::channel("alpha", 1);
    app.channels.insert("alpha".into(), ch.clone());
    server::setup(&app.io, app.clone());
    let http = test_support::HttpServer::start(axum::Router::new().layer(layer)).await;
    let mut player = SocketClient::connect(&http.url, "alpha", Some("play-secret")).await;
    let mut standby = SocketClient::connect(&http.url, "alpha", Some("play-secret")).await;
    let role = player.event("playbackRole").await;
    assert_eq!(role["active"], true);
    let epoch = role["epoch"].as_u64().expect("playbackRole trae epoch");
    assert_eq!(standby.event("playbackRole").await["active"], false);

    // El player vigente con su epoch sigue activo.
    player.emit("playerPing", json!({"epoch": epoch})).await;
    let pong = player.event("playerPong").await;
    assert_eq!(pong["active"], true);
    assert_eq!(pong["epoch"], epoch);

    // Epoch ajena o socket desplazado: inactivo, así la página deja de sonar.
    player.emit("playerPing", json!({"epoch": epoch + 1})).await;
    assert_eq!(player.event("playerPong").await["active"], false);
    standby.emit("playerPing", json!({"epoch": epoch})).await;
    assert_eq!(standby.event("playerPong").await["active"], false);

    // Tras un relevo, la epoch crece: el cliente viejo detecta que fue desplazado.
    player.packet("41".into()).await;
    standby.event("playerAvailable").await;
    let mut replacement = SocketClient::connect(&http.url, "alpha", Some("play-secret")).await;
    let role = replacement.event("playbackRole").await;
    assert_eq!(role["active"], true);
    assert!(role["epoch"].as_u64().unwrap() > epoch);
}

#[tokio::test]
#[ignore = "requires TEST_DATABASE_URL"]
async fn socket_authorization_player_takeover_and_channel_isolation() {
    let database = test_support::TestDatabase::new().await;
    let (layer, io) = socketioxide::SocketIo::new_layer();
    let mock = Arc::new(MockKick::default());
    let mock_server = mock.serve().await;
    let mut app = test_support::app_with_io(database.pool.clone(), io);
    use_mock(&mut app, &mock_server);
    let private = RsaPrivateKey::new(&mut rand::thread_rng(), 2048).unwrap();
    Arc::get_mut(&mut app).unwrap().webhook_key = RsaPublicKey::from(&private);
    let mut receivers = Vec::new();
    for (slug, id) in [("alpha", 1), ("beta", 2)] {
        db::upsert_channel(&database.pool, &test_support::row(slug, id))
            .await
            .unwrap();
        let (ch, receiver) = test_support::channel(slug, id as u64);
        receivers.push(receiver);
        app.channels.insert(slug.into(), ch);
        app.user_id_to_key
            .insert((Platform::Kick, id as u64), slug.into());
    }
    let alpha = app.channels.get("alpha").unwrap().clone();
    queue::change(&alpha, &app, |q| {
        for id in ["one", "two", "three"] {
            q.push(queue::VideoItem {
                id: id.into(),
                video_id: None,
                url: Some("https://cdn.example/video.mp4".into()),
                title: "same video".into(),
                user: "viewer".into(),
            });
        }
        true
    })
    .await
    .unwrap();
    server::setup(&app.io, app.clone());
    let http = test_support::HttpServer::start(axum::Router::new().layer(layer)).await;
    let mut public = SocketClient::connect(&http.url, "alpha", None).await;
    let mut wrong = SocketClient::connect(&http.url, "alpha", Some("wrong")).await;
    let mut player = SocketClient::connect(&http.url, "alpha", Some("play-secret")).await;
    let mut standby = SocketClient::connect(&http.url, "alpha", Some("play-secret")).await;
    let mut beta = SocketClient::connect(&http.url, "beta", None).await;
    assert_eq!(public.event("playbackRole").await["active"], false);
    assert_eq!(wrong.event("playbackRole").await["active"], false);
    assert_eq!(player.event("playbackRole").await["active"], true);
    assert_eq!(standby.event("playbackRole").await["active"], false);
    assert_eq!(
        public.event("syncQueue").await["items"]
            .as_array()
            .unwrap()
            .len(),
        3
    );
    assert!(beta.event("syncQueue").await["items"]
        .as_array()
        .unwrap()
        .is_empty());
    for socket in [&public, &wrong, &standby] {
        socket
            .emit("advanceQueue", json!({"id":"one","version":3}))
            .await;
        socket
            .emit(
                "panelCommand",
                json!({"command":"clearQueue", "token":"play-secret"}),
            )
            .await;
    }
    // The panel token also works on a viewer connection and remains separate from playback.
    public
        .emit(
            "panelCommand",
            json!({"command":"toggleVideo", "show":false,"token":"panel-secret"}),
        )
        .await;
    assert_eq!(public.event("toggleVideo").await["showVideo"], true); // initial snapshot
    assert_eq!(public.event("toggleVideo").await["showVideo"], false);
    assert_eq!(alpha.video_queue.read().await.items.len(), 3);
    player.event("syncQueue").await; // initial snapshot
    tokio::join!(
        player.emit("advanceQueue", json!({"id":"one","version":3})),
        player.emit("advanceQueue", json!({"id":"one","version":3}))
    );
    let advanced = player.event("syncQueue").await;
    assert_eq!(advanced["items"][0]["id"], "two");
    assert_eq!(advanced["version"], 4);

    // Signed owner identity governs commands even when a viewer spoofs the username.
    // Los moderadores del canal, identificados por su insignia firmada, también son owner.
    let worker = tokio::spawn(webhook::worker(app.clone()));
    // Los comandos no se muestran en el chat del overlay; los mensajes normales sí.
    for (event_id, sender_id, badge, content, expected_len, raffle_open) in [
        ("spoof", 99, "", "!skip", 2, false),
        ("vip", 98, "vip", "!sorteo abrir", 2, false),
        ("moderator", 97, "moderator", "!sorteo abrir", 2, true),
        ("owner", 1, "", "!skip", 1, true),
        ("chat", 99, "", "hola chat", 1, true),
    ] {
        let timestamp = chrono::Utc::now().to_rfc3339();
        let body = serde_json::to_vec(&json!({"broadcaster":{"user_id":1},"sender":{"user_id":sender_id,"username":"alpha","identity":{"badges":[{"type":badge}]}},"content":content})).unwrap();
        let mut message = format!("{event_id}.{timestamp}.").into_bytes();
        message.extend_from_slice(&body);
        let signature = SigningKey::<Sha256>::new(private.clone()).sign(&message);
        let headers = HeaderMap::from_iter([
            (
                "kick-event-message-id".parse().unwrap(),
                event_id.parse().unwrap(),
            ),
            (
                "kick-event-message-timestamp".parse().unwrap(),
                timestamp.parse().unwrap(),
            ),
            (
                "kick-event-type".parse().unwrap(),
                "chat.message.sent".parse().unwrap(),
            ),
            (
                "kick-event-signature".parse().unwrap(),
                STANDARD.encode(signature.to_bytes()).parse().unwrap(),
            ),
        ]);
        assert_eq!(
            webhook::receive(State(app.clone()), headers, Bytes::from(body)).await,
            StatusCode::OK
        );
        tokio::time::timeout(std::time::Duration::from_secs(3), async {
            while !sqlx::query_scalar::<_, bool>(
                "SELECT processed FROM webhook_events WHERE message_id=$1",
            )
            .bind(event_id)
            .fetch_one(&database.pool)
            .await
            .unwrap()
            {
                tokio::time::sleep(std::time::Duration::from_millis(10)).await;
            }
        })
        .await
        .unwrap();
        assert_eq!(alpha.video_queue.read().await.items.len(), expected_len);
        assert_eq!(alpha.sorteo.lock().await.open, raffle_open);
    }
    assert!(app
        .channels
        .get("beta")
        .unwrap()
        .video_queue
        .read()
        .await
        .items
        .is_empty());
    assert_eq!(public.event("chatMessage").await["content"], "hola chat");
    server::ns_emit(
        &app,
        &app.channels.get("beta").unwrap(),
        "testMarker",
        json!({}),
    );
    beta.event("testMarker").await;
    assert!(!beta
        .events
        .iter()
        .any(|event| event[0] == "chatMessage" || event[0] == "syncQueue"));

    player.packet("41".into()).await;
    standby.event("playerAvailable").await;
    let mut replacement = SocketClient::connect(&http.url, "alpha", Some("play-secret")).await;
    assert_eq!(replacement.event("playbackRole").await["active"], true);
    let row = db::load_channel(&database.pool, Platform::Kick, "alpha")
        .await
        .unwrap();
    channel::start_channel(row, app.clone()).await.unwrap();
    assert!(alpha.cancel.is_cancelled());
    let mut reconnected = SocketClient::connect(&http.url, "alpha", Some("play-secret")).await;
    assert_eq!(reconnected.event("playbackRole").await["active"], true);
    assert_eq!(
        reconnected.event("syncQueue").await["items"][0]["id"],
        "three"
    );
    assert_eq!(reconnected.event("toggleVideo").await["showVideo"], false);
    app.shutdown.cancel();
    worker.await.unwrap();
    database.cleanup().await;
}

#[tokio::test]
#[ignore = "requires TEST_DATABASE_URL"]
async fn rotated_tokens_recover_after_temporary_database_write_failure() {
    let database = test_support::TestDatabase::new().await;
    db::upsert_channel(&database.pool, &test_support::row("alpha", 1))
        .await
        .unwrap();
    let mock = Arc::new(MockKick::default());
    let service = mock.serve().await;
    let mut app = test_support::app(database.pool.clone());
    use_mock(&mut app, &service);
    let (ch, _receiver) = test_support::channel("alpha", 1);
    // Fail writes while leaving reads and the external token service available.
    sqlx::raw_sql("CREATE FUNCTION fail_write() RETURNS trigger LANGUAGE plpgsql AS $$ BEGIN RAISE EXCEPTION 'simulated outage'; END $$; CREATE TRIGGER outage BEFORE UPDATE ON channels FOR EACH ROW EXECUTE FUNCTION fail_write();")
        .execute(&database.pool).await.unwrap();
    assert!(!kick::refresh_access_token(&ch, &app).await);
    assert_eq!(*ch.refresh_token_val.read().await, "refresh-1");
    assert_eq!(
        db::load_channel(&database.pool, Platform::Kick, "alpha")
            .await
            .unwrap()
            .refresh_token,
        "refresh"
    );
    sqlx::query("DROP TRIGGER outage ON channels")
        .execute(&database.pool)
        .await
        .unwrap();
    assert!(kick::refresh_access_token(&ch, &app).await);
    assert_eq!(
        db::load_channel(&database.pool, Platform::Kick, "alpha")
            .await
            .unwrap()
            .refresh_token,
        "refresh-2"
    );
    assert_eq!(*ch.refresh_token_val.read().await, "refresh-2");
    database.cleanup().await;
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
            platform: "kick".into(),
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
        app.user_id_to_key
            .insert((Platform::Kick, id as u64), slug.into());
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
    let loaded = db::load_channel(&pool, Platform::Kick, "alpha")
        .await
        .unwrap();
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
    let reloaded = db::load_channel(&pool, Platform::Kick, "alpha")
        .await
        .unwrap();
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

/// Recorre todos los comandos de comandos.html como lo haría el chat real.
#[tokio::test]
#[ignore = "requires TEST_DATABASE_URL"]
async fn every_documented_chat_command_responds() {
    use crate::commands::handle_text as handle;
    let database = test_support::TestDatabase::new().await;
    db::upsert_channel(&database.pool, &test_support::row("alpha", 1))
        .await
        .unwrap();
    let mock = Arc::new(MockKick::default());
    let server = mock.serve().await;
    let mut app = test_support::app(database.pool.clone());
    use_mock(&mut app, &server);
    let (ch, mut tts) = test_support::channel("alpha", 1);
    app.channels.insert("alpha".into(), ch.clone());

    let last = || async {
        let requests = mock.chat_requests.lock().await;
        requests
            .last()
            .map(|(_, body)| body["content"].as_str().unwrap_or("").to_owned())
            .unwrap_or_default()
    };
    let count = || async { mock.chat_requests.lock().await.len() };
    macro_rules! says {
        ($user:expr, $msg:expr, $owner:expr, $expect:expr) => {{
            let before = count().await;
            handle($user, $msg, $owner, &ch, &app).await;
            assert!(count().await > before, "{} no respondió", $msg);
            let reply = last().await;
            assert!(reply.contains($expect), "{} respondió {reply:?}", $msg);
        }};
    }
    macro_rules! silent {
        ($user:expr, $msg:expr, $owner:expr) => {{
            let before = count().await;
            handle($user, $msg, $owner, &ch, &app).await;
            assert_eq!(count().await, before, "{} no debía responder", $msg);
        }};
    }

    // Información y configuración con !set
    says!("ana", "!comandos", false, "!s/!dalia/!jorge/!alex");
    says!("ana", "!voces", false, "Voces disponibles:");
    assert!(!last().await.contains("!jacinta"));
    silent!("pedro", "!voces", false);
    Arc::get_mut(&mut app).unwrap().config.fish_audio_api_key = "test-key".into();
    says!("dai", "!VOCES", true, "!jacinta");
    assert!(tts.try_recv().is_err());
    says!("ana", "!discord", false, "aún no configuró !discord");
    silent!("ana", "!set discord https://evil.example", false);
    says!(
        "dai",
        "!set discord https://discord.gg/abc",
        true,
        "✅ !discord actualizado"
    );
    says!("dai", "!discord", true, "Discord → https://discord.gg/abc");
    says!("dai", "!set redes x.com/dai", true, "✅ !redes actualizado");
    says!("dai", "!redes", true, "Redes → x.com/dai");
    says!(
        "dai",
        "!SET pc Ryzen 7 · RTX 4070",
        true,
        "✅ !pc actualizado"
    );
    says!("dai", "!setup", true, "Setup → Ryzen 7 · RTX 4070");
    says!(
        "dai",
        "!set horario Lun-Vie 8pm",
        true,
        "✅ !horario actualizado"
    );
    says!("dai", "!horario", true, "Horario → Lun-Vie 8pm");
    says!("dai", "!set meta 50", true, "Meta de seguidores: 50");
    says!("dai", "!set nada", true, "Uso: !set");
    let saved = db::load_channel(&database.pool, Platform::Kick, "alpha")
        .await
        .unwrap();
    assert_eq!(saved.cmd_discord, "https://discord.gg/abc");
    assert_eq!(saved.cmd_pc, "Ryzen 7 · RTX 4070");
    assert_eq!(saved.follow_goal, 50);
    ch.followers.store(5, Ordering::Relaxed);
    ch.followers_known.store(true, Ordering::Relaxed);
    says!("ana", "!seguidores", false, "5 / 50 (10%)");
    says!("ana", "!uptime", false, "offline");
    *ch.live_since.write().await = Some(chrono::Utc::now() - chrono::Duration::minutes(61));
    says!("dai", "!uptime", true, "En vivo: 1h 1m");

    // Entretenimiento
    says!("ana", "!dado", false, "ana sacó un");
    says!("ana", "!8ball ¿gano hoy?", false, "🎱");
    says!("dai", "!sorteo abrir", true, "sorteo está abierto");
    says!("ana", "!sorteo", false, "@ana se unió al sorteo! (1)");
    says!("beto", "!participar", false, "@beto se unió al sorteo! (2)");
    silent!("ana", "!participar", false);
    silent!("ana", "!sorteo ganador", false);
    says!("dai", "!sorteo cerrar", true, "2 participantes");
    says!("dai", "!sorteo ganador", true, "El ganador es @");

    // Videos y cola
    says!(
        "ana",
        "!Play https://cdn.example.com/a.mp4",
        false,
        "@ana agregó «a.mp4»"
    );
    says!(
        "beto",
        "!play https://cdn.example.com/b.mp4",
        false,
        "@beto agregó «b.mp4»"
    );
    says!("carla", "!cola", false, "1. a.mp4 · 2. b.mp4");
    says!("beto", "!misongs", false, "2. b.mp4");
    says!("beto", "!quitarme", false, "@beto retiro su video");
    assert_eq!(ch.video_queue.read().await.items.len(), 1);
    handle("ana", "!next", false, &ch, &app).await;
    assert_eq!(
        ch.video_queue.read().await.items.len(),
        1,
        "!next es solo del streamer"
    );

    // Solo streamer
    handle("dai", "!voff", true, &ch, &app).await;
    assert!(!ch.show_video.load(Ordering::Relaxed));
    handle("dai", "!von", true, &ch, &app).await;
    assert!(ch.show_video.load(Ordering::Relaxed));
    handle("dai", "!skip", true, &ch, &app).await;
    assert!(ch.video_queue.read().await.items.is_empty());
    handle(
        "dai",
        "!play https://cdn.example.com/c.mp4",
        true,
        &ch,
        &app,
    )
    .await;
    handle(
        "dai",
        "!play https://cdn.example.com/d.mp4",
        true,
        &ch,
        &app,
    )
    .await;
    assert_eq!(ch.video_queue.read().await.items.len(), 2);
    handle("dai", "!vstop", true, &ch, &app).await;
    assert!(ch.video_queue.read().await.items.is_empty());

    // Texto a voz
    for (user, msg, voice) in [
        ("u1", "!camila hola", "camila"),
        ("u2", "!dalia hola", "dalia"),
        ("u3", "!jorge hola", "jorge"),
        ("u4", "!alex hola", "alex"),
        ("u5", "!narrador hola", "narrador"),
        ("u6", "!epico hola", "epico"),
        ("u7", "!comedia hola", "comedia"),
        ("u8", "!jacinta hola", "jacinta"),
    ] {
        handle(user, msg, false, &ch, &app).await;
        let item = tts
            .try_recv()
            .unwrap_or_else(|_| panic!("{msg} no generó TTS"));
        assert_eq!(
            (item.voice.as_str(), item.text.as_str()),
            (voice, format!("{user} dice: hola").as_str())
        );
    }
    handle("u9", "!s hola", false, &ch, &app).await;
    let item = tts.try_recv().expect("!s no generó TTS");
    assert!(crate::tts::DEFAULT_CHAT_VOICES.contains(&item.voice.as_str()));

    drop(server);
    database.cleanup().await;
}

#[tokio::test]
#[ignore = "requires TEST_DATABASE_URL"]
async fn kick_and_twitch_channels_with_the_same_name_stay_separate() {
    let database = test_support::TestDatabase::new().await;
    let pool = &database.pool;
    let kick_row = test_support::row("pepe", 1);
    let twitch_row = db::ChannelRow {
        platform: "twitch".into(),
        cmd_discord: "discord de twitch".into(),
        ..test_support::row("pepe", 1)
    };
    db::upsert_channel(pool, &kick_row).await.unwrap();
    db::upsert_channel(pool, &twitch_row).await.unwrap();
    assert_eq!(db::load_all_channels(pool).await.unwrap().len(), 2);
    assert_eq!(
        db::load_channel(pool, Platform::Kick, "pepe")
            .await
            .unwrap()
            .cmd_discord,
        "saved-setting"
    );
    assert_eq!(
        db::load_channel(pool, Platform::Twitch, "pepe")
            .await
            .unwrap()
            .cmd_discord,
        "discord de twitch"
    );

    // Comandos, metas, tokens y baneos solo cambian la fila de su plataforma.
    db::update_command_text(pool, Platform::Twitch, "pepe", "pc", "pc twitch")
        .await
        .unwrap();
    db::update_follow_goal(pool, Platform::Twitch, "pepe", 500)
        .await
        .unwrap();
    assert!(db::update_tokens(
        pool,
        Platform::Twitch,
        "pepe",
        "tw-access",
        "tw-refresh",
        9,
        "refresh"
    )
    .await
    .unwrap());
    let kick = db::load_channel(pool, Platform::Kick, "pepe")
        .await
        .unwrap();
    let twitch = db::load_channel(pool, Platform::Twitch, "pepe")
        .await
        .unwrap();
    assert_eq!((kick.cmd_pc.as_str(), kick.follow_goal), ("", 200));
    assert_eq!(kick.access_token, "access");
    assert_eq!(
        (twitch.cmd_pc.as_str(), twitch.follow_goal),
        ("pc twitch", 500)
    );
    assert_eq!(twitch.access_token, "tw-access");
    assert!(db::add_ban(pool, Platform::Twitch, "pepe", "troll")
        .await
        .unwrap());
    assert!(db::load_bans(pool, Platform::Kick, "pepe")
        .await
        .unwrap()
        .is_empty());
    assert_eq!(
        db::load_bans(pool, Platform::Twitch, "pepe").await.unwrap(),
        ["troll"]
    );
    assert!(!db::remove_ban(pool, Platform::Kick, "pepe", "troll")
        .await
        .unwrap());

    // Cada canal tiene su propia clave en memoria y su propia sala de overlays.
    let app = test_support::app(pool.clone());
    channel::start_channel(kick, app.clone()).await.unwrap();
    channel::start_channel(twitch, app.clone()).await.unwrap();
    assert_eq!(app.channels.get("pepe").unwrap().platform, Platform::Kick);
    assert_eq!(
        app.channels.get("twitch:pepe").unwrap().platform,
        Platform::Twitch
    );
    assert_eq!(
        app.user_id_to_key
            .get(&(Platform::Twitch, 1))
            .unwrap()
            .value(),
        "twitch:pepe"
    );
    assert_eq!(
        app.user_id_to_key
            .get(&(Platform::Kick, 1))
            .unwrap()
            .value(),
        "pepe"
    );

    // El inbox acepta el mismo ID de entrega en las dos plataformas.
    for platform in ["kick", "twitch"] {
        sqlx::query(
            "INSERT INTO webhook_events (platform,message_id,received_at,payload,event_type) VALUES ($1,'same-id',0,'{}','x')",
        )
        .bind(platform)
        .execute(pool)
        .await
        .unwrap();
    }
    assert!(sqlx::query(
        "INSERT INTO channels (platform, slug, access_token) VALUES ('youtube','x','t')"
    )
    .execute(pool)
    .await
    .is_err());
    app.shutdown.cancel();
    database.cleanup().await;
}

/// Twitch simulado: tokens, validación y `/helix/users` con un usuario configurable.
#[derive(Default)]
struct MockTwitch {
    forms: Mutex<Vec<HashMap<String, String>>>,
    token_requests: AtomicUsize,
    user: Mutex<Option<Value>>,
    validate_status: Mutex<Option<StatusCode>>,
    validations: AtomicUsize,
    subscriptions: Mutex<Vec<(String, Value)>>,
    subscription_status: Mutex<Option<StatusCode>>,
}

impl MockTwitch {
    async fn serve(self: &Arc<Self>) -> test_support::HttpServer {
        async fn token(
            State(mock): State<Arc<MockTwitch>>,
            axum::Form(form): axum::Form<HashMap<String, String>>,
        ) -> axum::Json<Value> {
            mock.forms.lock().await.push(form);
            let n = mock.token_requests.fetch_add(1, Ordering::Relaxed) + 1;
            axum::Json(
                json!({"access_token":format!("tw-access-{n}"),"refresh_token":format!("tw-refresh-{n}"),"expires_in":14000}),
            )
        }
        async fn validate(State(mock): State<Arc<MockTwitch>>) -> StatusCode {
            mock.validations.fetch_add(1, Ordering::Relaxed);
            mock.validate_status.lock().await.unwrap_or(StatusCode::OK)
        }
        async fn users(
            State(mock): State<Arc<MockTwitch>>,
            headers: HeaderMap,
        ) -> axum::Json<Value> {
            assert_eq!(headers.get("client-id").unwrap(), "tw-client");
            axum::Json(json!({"data":[mock.user.lock().await.clone().unwrap()]}))
        }
        async fn subscribe(
            State(mock): State<Arc<MockTwitch>>,
            headers: HeaderMap,
            axum::Json(body): axum::Json<Value>,
        ) -> StatusCode {
            let token = headers["authorization"].to_str().unwrap().to_owned();
            mock.subscriptions.lock().await.push((token, body));
            mock.subscription_status
                .lock()
                .await
                .unwrap_or(StatusCode::ACCEPTED)
        }
        test_support::HttpServer::start(
            axum::Router::new()
                .route(
                    "/helix/eventsub/subscriptions",
                    axum::routing::post(subscribe),
                )
                .route("/oauth2/token", axum::routing::post(token))
                .route("/oauth2/validate", axum::routing::get(validate))
                .route("/helix/users", axum::routing::get(users))
                .with_state(self.clone()),
        )
        .await
    }

    async fn set_user(&self, id: &str, login: &str) {
        *self.user.lock().await = Some(json!({
            "id": id, "login": login, "display_name": login.to_uppercase(),
            "profile_image_url": "https://static-cdn.jtvnw.net/avatar.png"
        }));
    }
}

/// Recorre `/auth/twitch…` como un navegador: redirige, guarda la cookie y vuelve con un código.
async fn twitch_authorize(
    app: &Arc<crate::state::AppState>,
    bot: bool,
) -> (HeaderMap, HashMap<String, String>) {
    use axum::{http::header, response::IntoResponse};
    let response = if bot {
        crate::twitch::auth::redirect_bot(State(app.clone())).await
    } else {
        crate::twitch::auth::redirect_channel(State(app.clone())).await
    }
    .into_response();
    let location = response.headers()[header::LOCATION]
        .to_str()
        .unwrap()
        .to_owned();
    let url = url::Url::parse(&location).unwrap();
    let query: HashMap<_, _> = url.query_pairs().into_owned().collect();
    assert_eq!(query["client_id"], "tw-client");
    assert_eq!(query["force_verify"], "true");
    let callback = if bot {
        "/auth/twitch/bot/callback"
    } else {
        "/auth/twitch/callback"
    };
    assert_eq!(
        query["redirect_uri"],
        format!("https://test.example{callback}")
    );
    let cookie = response.headers()[header::SET_COOKIE].to_str().unwrap();
    let mut headers = HeaderMap::new();
    headers.insert(
        header::COOKIE,
        cookie.split(';').next().unwrap().parse().unwrap(),
    );
    let params = HashMap::from([
        ("code".to_string(), "the-code".to_string()),
        ("state".to_string(), query["state"].clone()),
    ]);
    (headers, params)
}

#[tokio::test]
#[ignore = "requires TEST_DATABASE_URL"]
async fn twitch_channels_and_the_bot_account_connect_and_renew_tokens() {
    use crate::twitch::{auth as tw, bot};
    use axum::extract::Query;
    let database = test_support::TestDatabase::new().await;
    let mock = Arc::new(MockTwitch::default());
    let service = mock.serve().await;
    let mut app = test_support::app(database.pool.clone());
    Arc::get_mut(&mut app).unwrap().twitch_endpoints = crate::twitch::Endpoints {
        oauth: format!("{}/oauth2", service.url),
        api: format!("{}/helix", service.url),
    };
    // Un canal de Kick llamado igual no se ve afectado.
    db::upsert_channel(&database.pool, &test_support::row("ana", 5))
        .await
        .unwrap();

    // 1. El streamer conecta su canal de Twitch.
    mock.set_user("5", "ana").await;
    let (headers, params) = twitch_authorize(&app, false).await;
    let page = tw::channel_callback(State(app.clone()), headers.clone(), Query(params.clone()))
        .await
        .0;
    assert!(
        page.contains("¡DaiBot conectado para <b>ana</b>!"),
        "{page}"
    );
    assert!(page.contains("https://test.example/vsaturn.html?tw=ana#token="));
    assert!(page.contains("/dock-chat.html?tw=ana#token="));
    assert!(page.contains(r#"<fieldset class="themes" hidden>"#));
    assert!(page.contains("twitch.tv/ana"));
    let form = mock.forms.lock().await.last().unwrap().clone();
    assert_eq!(form["grant_type"], "authorization_code");
    assert_eq!(form["client_secret"], "tw-secret");
    assert_eq!(
        form["redirect_uri"],
        "https://test.example/auth/twitch/callback"
    );
    let twitch = db::load_channel(&database.pool, Platform::Twitch, "ana")
        .await
        .unwrap();
    assert_eq!(twitch.broadcaster_user_id, Some(5));
    assert_eq!(twitch.access_token, "tw-access-1");
    let ch = app.channels.get("twitch:ana").unwrap().clone();
    assert_eq!(ch.platform, Platform::Twitch);
    assert_eq!(*ch.display_name.read().await, "ANA");
    assert_eq!(
        db::load_channel(&database.pool, Platform::Kick, "ana")
            .await
            .unwrap()
            .access_token,
        "access"
    );
    // El mismo `state` no se puede reutilizar.
    let replay = tw::channel_callback(State(app.clone()), headers, Query(params))
        .await
        .0;
    assert!(replay.contains("venció"), "{replay}");

    // 2. Un `state` del flujo del bot no sirve para conectar un canal, ni la cuenta bot como canal.
    let (headers, params) = twitch_authorize(&app, true).await;
    let crossed = tw::channel_callback(State(app.clone()), headers, Query(params))
        .await
        .0;
    assert!(crossed.contains("venció"), "{crossed}");
    mock.set_user(bot::BOT_USER_ID, "daibotsito").await;
    let (headers, params) = twitch_authorize(&app, false).await;
    let as_bot = tw::channel_callback(State(app.clone()), headers, Query(params))
        .await
        .0;
    assert!(as_bot.contains("cuenta del bot"), "{as_bot}");

    // 3. Solo daibotsito puede autorizarse como bot.
    mock.set_user("701010126", "sr_dai_twitch").await;
    let (headers, params) = twitch_authorize(&app, true).await;
    let wrong = tw::bot_callback(State(app.clone()), headers, Query(params))
        .await
        .0;
    assert!(wrong.contains("Entraste como sr_dai_twitch"), "{wrong}");
    assert!(app.twitch_bot.read().await.is_none());
    mock.set_user(bot::BOT_USER_ID, "daibotsito").await;
    let (headers, params) = twitch_authorize(&app, true).await;
    let ok = tw::bot_callback(State(app.clone()), headers, Query(params))
        .await
        .0;
    assert!(ok.contains("daibotsito"), "{ok}");
    let saved = db::load_bot_account(&database.pool, Platform::Twitch)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(saved.user_id, bot::BOT_USER_ID);
    assert_eq!(
        app.twitch_bot.read().await.as_ref().unwrap().access_token,
        saved.access_token
    );

    // 4. Si el streamer cambia su login, el canal se renombra y conserva baneos y ajustes.
    db::add_ban(&database.pool, Platform::Twitch, "ana", "troll")
        .await
        .unwrap();
    db::update_command_text(&database.pool, Platform::Twitch, "ana", "pc", "mi pc")
        .await
        .unwrap();
    mock.set_user("5", "ana_nueva").await;
    let (headers, params) = twitch_authorize(&app, false).await;
    let renamed = tw::channel_callback(State(app.clone()), headers, Query(params))
        .await
        .0;
    assert!(
        renamed.contains("vsaturn.html?tw=ana_nueva#token="),
        "{renamed}"
    );
    assert!(db::load_channel(&database.pool, Platform::Twitch, "ana")
        .await
        .is_err());
    let moved = db::load_channel(&database.pool, Platform::Twitch, "ana_nueva")
        .await
        .unwrap();
    assert_eq!(moved.cmd_pc, "mi pc");
    assert_eq!(moved.panel_token, twitch.panel_token);
    assert_eq!(
        db::load_bans(&database.pool, Platform::Twitch, "ana_nueva")
            .await
            .unwrap(),
        ["troll"]
    );
    assert!(app.channels.get("twitch:ana").is_none());
    assert!(ch.cancel.is_cancelled());

    // 5. Los tokens de Twitch se renuevan con las credenciales de la app de Twitch.
    let ch = app.channels.get("twitch:ana_nueva").unwrap().clone();
    let before = mock.token_requests.load(Ordering::Relaxed);
    assert!(crate::oauth::refresh_access_token(&ch, &app).await);
    let form = mock.forms.lock().await.last().unwrap().clone();
    assert_eq!(form["grant_type"], "refresh_token");
    assert_eq!(form["client_id"], "tw-client");
    let refreshed = db::load_channel(&database.pool, Platform::Twitch, "ana_nueva")
        .await
        .unwrap();
    assert_eq!(refreshed.access_token, format!("tw-access-{}", before + 1));
    assert_eq!(*ch.access_token.read().await, refreshed.access_token);
    assert_eq!(
        crate::twitch::validate(&app, &refreshed.access_token).await,
        crate::twitch::Validation::Valid
    );
    *mock.validate_status.lock().await = Some(StatusCode::UNAUTHORIZED);
    assert_eq!(
        crate::twitch::validate(&app, "revoked").await,
        crate::twitch::Validation::Invalid
    );
    app.shutdown.cancel();
    database.cleanup().await;
}

/// Firma un envío de EventSub como Twitch (HMAC-SHA256 de id + timestamp + cuerpo).
fn twitch_signed(secret: &str, id: &str, kind: &str, body: &Value) -> (HeaderMap, Bytes) {
    use hmac::{Hmac, Mac};
    let body = Bytes::from(body.to_string());
    let timestamp = chrono::Utc::now().to_rfc3339();
    let mut mac = Hmac::<Sha256>::new_from_slice(secret.as_bytes()).unwrap();
    mac.update(id.as_bytes());
    mac.update(timestamp.as_bytes());
    mac.update(&body);
    let hex: String = mac
        .finalize()
        .into_bytes()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    let mut headers = HeaderMap::new();
    headers.insert("Twitch-Eventsub-Message-Id", id.parse().unwrap());
    headers.insert(
        "Twitch-Eventsub-Message-Timestamp",
        timestamp.parse().unwrap(),
    );
    headers.insert(
        "Twitch-Eventsub-Message-Signature",
        format!("sha256={hex}").parse().unwrap(),
    );
    headers.insert("Twitch-Eventsub-Message-Type", kind.parse().unwrap());
    (headers, body)
}

#[tokio::test]
#[ignore = "requires TEST_DATABASE_URL"]
async fn twitch_eventsub_subscribes_and_delivers_chat_and_alerts() {
    use crate::twitch::{bot, eventsub, ingest};
    use axum::response::IntoResponse;
    let database = test_support::TestDatabase::new().await;
    let mock = Arc::new(MockTwitch::default());
    let service = mock.serve().await;
    let mut app = test_support::app(database.pool.clone());
    Arc::get_mut(&mut app).unwrap().twitch_endpoints = crate::twitch::Endpoints {
        oauth: format!("{}/oauth2", service.url),
        api: format!("{}/helix", service.url),
    };
    let row = db::ChannelRow {
        platform: "twitch".into(),
        ..test_support::row("ana", 5)
    };
    db::upsert_channel(&database.pool, &row).await.unwrap();
    // Un canal de Kick con el mismo ID de broadcaster no recibe eventos de Twitch.
    db::upsert_channel(&database.pool, &test_support::row("ana", 5))
        .await
        .unwrap();
    for row in db::load_all_channels(&database.pool).await.unwrap() {
        channel::start_channel(row, app.clone()).await.unwrap();
    }
    let ch = app.channels.get("twitch:ana").unwrap().clone();
    let kick = app.channels.get("ana").unwrap().clone();

    // 1. Sin la cuenta bot se crean las alertas, pero no el chat, y se reintentará.
    assert!(!eventsub::subscribe(&ch, &app).await);
    let subs = mock.subscriptions.lock().await.clone();
    assert_eq!(subs.len(), 6);
    assert!(subs
        .iter()
        .all(|(token, body)| token == "Bearer tw-access-1"
            && body["transport"]["callback"] == "https://test.example/twitch_webhook"
            && body["transport"]["secret"] == "tw-eventsub-secret"
            && body["condition"]["broadcaster_user_id"] == "5"));
    assert_eq!(
        mock.forms.lock().await[0]["grant_type"],
        "client_credentials"
    );
    // 2. Con el bot autorizado se suscribe al chat; el token de app se reutiliza y 409 cuenta como hecho.
    *app.twitch_bot.write().await = Some(bot::BotAccount {
        user_id: bot::BOT_USER_ID.into(),
        login: "daibotsito".into(),
        access_token: "bot-access".into(),
        refresh_token: "bot-refresh".into(),
        token_expires: i64::MAX,
    });
    *mock.subscription_status.lock().await = Some(StatusCode::CONFLICT);
    assert!(eventsub::subscribe(&ch, &app).await);
    let subs = mock.subscriptions.lock().await.clone();
    let chat = subs
        .iter()
        .find(|(_, b)| b["type"] == "channel.chat.message")
        .unwrap();
    assert_eq!(chat.1["condition"]["user_id"], bot::BOT_USER_ID);
    assert_eq!(mock.token_requests.load(Ordering::Relaxed), 1);

    // 3. El webhook verifica la firma, responde el challenge y guarda notificaciones una sola vez.
    let secret = "tw-eventsub-secret";
    let (headers, body) = twitch_signed(
        secret,
        "verify-1",
        "webhook_callback_verification",
        &json!({"challenge":"abc123","subscription":{"type":"stream.online"}}),
    );
    let response = eventsub::receive(State(app.clone()), headers, body)
        .await
        .into_response();
    assert_eq!(response.status(), StatusCode::OK);
    let text = axum::body::to_bytes(response.into_body(), 1024)
        .await
        .unwrap();
    assert_eq!(&text[..], b"abc123");
    let chat_event = |id: &str, user_id: &str, login: &str, text: &str| {
        json!({"subscription":{"type":"channel.chat.message"},"event":{
            "broadcaster_user_id":"5","chatter_user_id":user_id,"chatter_user_login":login,
            "chatter_user_name":login,"message":{"text":text},"badges":[],"message_id":id}})
    };
    let payload = chat_event("m1", "9", "viewer", "!participar");
    for _ in 0..2 {
        let (headers, body) = twitch_signed(secret, "delivery-1", "notification", &payload);
        let status = eventsub::receive(State(app.clone()), headers, body)
            .await
            .into_response()
            .status();
        assert_eq!(status, StatusCode::NO_CONTENT);
    }
    let stored: Vec<(String, String)> =
        sqlx::query_as("SELECT platform, event_type FROM webhook_events")
            .fetch_all(&database.pool)
            .await
            .unwrap();
    assert_eq!(
        stored,
        [("twitch".to_string(), "channel.chat.message".to_string())]
    );
    assert_eq!(app.metrics.webhook_duplicates.load(Ordering::Relaxed), 1);
    let (headers, body) = twitch_signed("wrong-secret", "delivery-2", "notification", &payload);
    let status = eventsub::receive(State(app.clone()), headers, body)
        .await
        .into_response()
        .status();
    assert_eq!(status, StatusCode::FORBIDDEN);

    // 4. Chat y comandos: el sorteo ve a los espectadores de Twitch; el eco del bot se ignora.
    ch.sorteo.lock().await.open = true;
    kick.sorteo.lock().await.open = true;
    ingest::process(&app, "channel.chat.message", &payload).await;
    ingest::process(
        &app,
        "channel.chat.message",
        &chat_event("m2", bot::BOT_USER_ID, "daibotsito", "!participar"),
    )
    .await;
    assert_eq!(ch.sorteo.lock().await.participants, ["viewer"]);
    assert!(kick.sorteo.lock().await.participants.is_empty());
    // Un baneado de Twitch no entra al sorteo.
    ch.banned.write().await.insert("troll".into());
    ingest::process(
        &app,
        "channel.chat.message",
        &chat_event("m3", "10", "troll", "!participar"),
    )
    .await;
    assert_eq!(ch.sorteo.lock().await.participants, ["viewer"]);

    // 5. Directo: stream.online y stream.offline alimentan !uptime.
    let event = |kind: &str, event: Value| json!({"subscription":{"type":kind},"event":event});
    ingest::process(
        &app,
        "stream.online",
        &event(
            "stream.online",
            json!({
        "broadcaster_user_id":"5","started_at":"2026-11-15T18:00:00Z"}),
        ),
    )
    .await;
    assert_eq!(
        ch.live_since.read().await.unwrap().to_rfc3339(),
        "2026-11-15T18:00:00+00:00"
    );
    assert!(kick.live_since.read().await.is_none());
    ingest::process(
        &app,
        "stream.offline",
        &event("stream.offline", json!({"broadcaster_user_id":"5"})),
    )
    .await;
    assert!(ch.live_since.read().await.is_none());
    app.shutdown.cancel();
    database.cleanup().await;
}
