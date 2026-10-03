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
            .filter(|page| page.contains("¡GorilinRix conectado"))
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
    app.user_id_to_slug.insert(1, "alpha".into());
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
    restarted.user_id_to_slug.insert(1, "alpha".into());
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
        app.user_id_to_slug.insert(id as u64, slug.into());
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
    let worker = tokio::spawn(webhook::worker(app.clone()));
    for (event_id, sender_id, expected_len) in [("spoof", 99, 2), ("owner", 1, 1)] {
        let timestamp = chrono::Utc::now().to_rfc3339();
        let body = serde_json::to_vec(&json!({"broadcaster":{"user_id":1},"sender":{"user_id":sender_id,"username":"alpha"},"content":"!skip"})).unwrap();
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
    assert_eq!(public.event("chatMessage").await["content"], "!skip");
    server::ns_emit(&app, "beta", "testMarker", json!({}));
    beta.event("testMarker").await;
    assert!(!beta
        .events
        .iter()
        .any(|event| event[0] == "chatMessage" || event[0] == "syncQueue"));

    player.packet("41".into()).await;
    standby.event("playerAvailable").await;
    let mut replacement = SocketClient::connect(&http.url, "alpha", Some("play-secret")).await;
    assert_eq!(replacement.event("playbackRole").await["active"], true);
    let row = db::load_channel(&database.pool, "alpha").await.unwrap();
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
        db::load_channel(&database.pool, "alpha")
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
        db::load_channel(&database.pool, "alpha")
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

/// Recorre todos los comandos de comandos.html como lo haría el chat real.
#[tokio::test]
#[ignore = "requires TEST_DATABASE_URL"]
async fn every_documented_chat_command_responds() {
    use crate::commands::handle;
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
    let saved = db::load_channel(&database.pool, "alpha").await.unwrap();
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
        ("u1", "!s hola", "camila"),
        ("u2", "!dalia hola", "dalia"),
        ("u3", "!jorge hola", "jorge"),
        ("u4", "!alex hola", "alex"),
    ] {
        handle(user, msg, false, &ch, &app).await;
        let item = tts
            .try_recv()
            .unwrap_or_else(|_| panic!("{msg} no generó TTS"));
        assert_eq!((item.voice.as_str(), item.text.as_str()), (voice, "hola"));
    }

    drop(server);
    database.cleanup().await;
}
