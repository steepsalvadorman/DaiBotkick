use crate::{
    kick::sender,
    server::ns_emit,
    state::{AppState, ChannelState},
};
use rand::seq::SliceRandom;
use std::{collections::HashMap, sync::Arc, time::Duration};

pub type Card = [u8; 25];

#[derive(Default)]
pub struct Bingo {
    round: u64,
    phase: &'static str,
    cards: HashMap<String, Card>,
    card_tokens: HashMap<String, String>,
    token_users: HashMap<String, String>,
    remaining: Vec<u8>,
    drawn: Vec<u8>,
    winner: Option<String>,
    line: Vec<u8>,
}

fn generate_card() -> Card {
    let mut card = [0; 25];
    for col in 0..5 {
        let mut numbers: Vec<u8> = (col as u8 * 15 + 1..=col as u8 * 15 + 15).collect();
        numbers.shuffle(&mut rand::thread_rng());
        for row in 0..5 {
            card[row * 5 + col] = numbers[row];
        }
    }
    card[12] = 0;
    card
}

fn winning_line(card: &Card, drawn: &[u8]) -> Option<Vec<u8>> {
    let mut lines: Vec<Vec<usize>> = (0..5)
        .map(|r| (0..5).map(|c| r * 5 + c).collect())
        .collect();
    lines.extend((0..5).map(|c| (0..5).map(|r| r * 5 + c).collect::<Vec<_>>()));
    lines.push(vec![0, 6, 12, 18, 24]);
    lines.push(vec![4, 8, 12, 16, 20]);
    lines
        .into_iter()
        .find(|line| {
            line.iter()
                .all(|&i| card[i] == 0 || drawn.contains(&card[i]))
        })
        .map(|line| line.into_iter().map(|i| card[i]).collect())
}

impl Bingo {
    pub fn snapshot(&self) -> serde_json::Value {
        serde_json::json!({
            "round": self.round, "phase": if self.phase.is_empty() { "idle" } else { self.phase },
            "drawn": self.drawn, "participants": self.cards.len(), "winner": self.winner,
            "line": self.line, "card": self.winner.as_ref().and_then(|user| self.cards.get(&user.to_lowercase()))
        })
    }

    fn open(&mut self) -> Result<(), &'static str> {
        if matches!(self.phase, "open" | "running" | "exhausted") {
            return Err("Ya hay un bingo abierto. Usa !bingo cancelar antes de crear otro.");
        }
        self.round += 1;
        self.phase = "open";
        self.cards.clear();
        self.card_tokens.clear();
        self.token_users.clear();
        self.drawn.clear();
        self.winner = None;
        self.line.clear();
        self.remaining = (1..=75).collect();
        self.remaining.shuffle(&mut rand::thread_rng());
        Ok(())
    }

    fn register(&mut self, user: &str) -> Result<Card, &'static str> {
        let key = user.to_lowercase();
        if let Some(card) = self.cards.get(&key) {
            return Ok(*card);
        }
        if self.phase != "open" {
            return Err("Inscripciones cerradas. Espera a que se abra el siguiente bingo.");
        }
        if self.cards.len() >= 1000 {
            return Err("El bingo ya tiene 1000 participantes.");
        }
        let card = generate_card();
        let token = uuid::Uuid::new_v4().simple().to_string();
        self.card_tokens.insert(key.clone(), token.clone());
        self.token_users.insert(token, key.clone());
        self.cards.insert(key, card);
        Ok(card)
    }

    fn card_link(&self, user: &str, base_url: &str, slug: &str) -> String {
        let token = &self.card_tokens[&user.to_lowercase()];
        let fragment = url::form_urlencoded::Serializer::new(String::new())
            .append_pair("ch", slug)
            .append_pair("token", token)
            .finish();
        format!("{}/bingo.html#{fragment}", base_url.trim_end_matches('/'))
    }

    fn personal_snapshot(&self, token: &str) -> Option<serde_json::Value> {
        let user = self.token_users.get(token)?;
        let card = self.cards.get(user)?;
        Some(serde_json::json!({
            "user": user, "round": self.round, "phase": self.phase,
            "card": card, "drawn": self.drawn, "winner": self.winner,
            "ready": matches!(self.phase, "running" | "exhausted")
                && winning_line(card, &self.drawn).is_some()
        }))
    }

    fn start(&mut self) -> Result<u64, &'static str> {
        if self.phase != "open" || self.cards.is_empty() {
            return Err("Abre el bingo y registra al menos un cartón antes de iniciar.");
        }
        self.phase = "running";
        Ok(self.round)
    }

    fn draw(&mut self, round: u64) -> Option<u8> {
        if self.round != round || self.phase != "running" {
            return None;
        }
        let number = self.remaining.pop()?;
        self.drawn.push(number);
        if self.remaining.is_empty() {
            self.phase = "exhausted";
        }
        Some(number)
    }

    fn claim(&mut self, user: &str) -> Result<(), &'static str> {
        if !matches!(self.phase, "running" | "exhausted") {
            return Err("No hay un bingo en curso que puedas reclamar.");
        }
        let card = self
            .cards
            .get(&user.to_lowercase())
            .ok_or("No tienes un cartón registrado en esta partida.")?;
        let line = winning_line(card, &self.drawn).ok_or(
            "Todavía no tienes una fila, columna o diagonal completa con las bolas sorteadas.",
        )?;
        self.line = line;
        self.winner = Some(user.to_owned());
        self.phase = "won";
        Ok(())
    }

    fn command(
        &mut self,
        user: &str,
        args: &str,
        owner: bool,
    ) -> Result<(String, Option<u64>), &'static str> {
        if matches!(args, "abrir" | "iniciar" | "cancelar") && !owner {
            return Err("Solo el dueño del canal puede abrir, iniciar o cancelar el bingo.");
        }
        match args {
            "abrir" => {
                self.open()?;
                Ok(("Bingo abierto: escribe !carton para recibir tu cartón. Victoria por fila, columna o diagonal.".into(), None))
            }
            "carton" => {
                self.register(user)?;
                Ok(("Cartón registrado.".into(), None))
            }
            "iniciar" => {
                let round = self.start()?;
                Ok(("Bingo iniciado. Inscripciones cerradas; una bola cada 10 segundos. Reclama tu línea con !bingo.".into(), Some(round)))
            }
            "cancelar" => {
                self.phase = "idle";
                self.round += 1;
                self.cards.clear();
                self.card_tokens.clear();
                self.token_users.clear();
                self.drawn.clear();
                self.remaining.clear();
                self.winner = None;
                self.line.clear();
                Ok(("Bingo cancelado.".into(), None))
            }
            "" => {
                self.claim(user)?;
                Ok((format!("BINGO: {user} gana con una línea verificada automáticamente. ¡Felicidades!"), None))
            }
            _ => Err("Usa !carton o !bingo. Dueño: !bingo abrir|iniciar|cancelar (!ruleta abre el bingo)."),
        }
    }
}

pub async fn handle(
    user: &str,
    command: &str,
    owner: bool,
    ch: &Arc<ChannelState>,
    app: &Arc<AppState>,
) -> bool {
    let args = if command == "!ruleta" {
        "abrir"
    } else if command == "!carton" {
        "carton"
    } else if command == "!bingo" {
        ""
    } else if let Some(args) = command.strip_prefix("!bingo ") {
        args.trim()
    } else {
        return false;
    };
    if !owner && !ch.cooldown.lock().await.consume_user(user, "!bingo", 5) {
        return true;
    }
    let mut game = ch.bingo.lock().await;
    let result = game.command(user, args, owner).map(|(message, round)| {
        if args == "carton" {
            (format!(
                "{user}, abre tu cartón y marca tus números aquí: {} . Para reclamar una línea, escribe !bingo en este chat. Quien tenga el enlace puede ver tu cartón.",
                game.card_link(user, &app.config.base_url, &ch.slug)
            ), round)
        } else {
            (message, round)
        }
    });
    let start = result.as_ref().ok().and_then(|(_, round)| *round);
    if result.is_ok() {
        ns_emit(app, &ch.slug, "bingoState", game.snapshot());
    }
    drop(game);
    if let Some(round) = start {
        let (ch, app) = (ch.clone(), app.clone());
        tokio::spawn(async move {
            let mut ticks = tokio::time::interval(Duration::from_secs(10));
            ticks.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
            ticks.tick().await;
            loop {
                tokio::select! {
                    _ = ch.cancel.cancelled() => break,
                    _ = ticks.tick() => {}
                }
                let mut game = ch.bingo.lock().await;
                let Some(number) = game.draw(round) else {
                    break;
                };
                ns_emit(&app, &ch.slug, "bingoState", game.snapshot());
                let exhausted = game.phase == "exhausted";
                drop(game);
                let letter = ["B", "I", "N", "G", "O"][usize::from((number - 1) / 15)];
                sender::send(
                    &format!(
                        "Bingo: {letter}-{number}.{}",
                        if exhausted {
                            " Han salido todas las bolas; reclama con !bingo."
                        } else {
                            ""
                        }
                    ),
                    &ch,
                    &app,
                )
                .await;
                if exhausted {
                    break;
                }
            }
        });
    }
    let message = result
        .map(|(message, _)| message)
        .unwrap_or_else(str::to_owned);
    sender::send(&message, ch, app).await;
    true
}

pub async fn personal_card(
    axum::extract::State(app): axum::extract::State<Arc<AppState>>,
    axum::extract::Path(slug): axum::extract::Path<String>,
    headers: axum::http::HeaderMap,
) -> axum::response::Response {
    use axum::{http::StatusCode, response::IntoResponse};
    let token = headers
        .get("x-bingo-token")
        .and_then(|value| value.to_str().ok());
    let Some(token) = token
        .filter(|token| token.len() == 32 && token.bytes().all(|byte| byte.is_ascii_hexdigit()))
    else {
        return (
            StatusCode::NOT_FOUND,
            "Cartón no disponible. Usa !carton en el chat.",
        )
            .into_response();
    };
    let channel = app.channels.get(&slug).map(|entry| entry.clone());
    let snapshot = if let Some(channel) = channel {
        if channel.cancel.is_cancelled() {
            None
        } else {
            channel.bingo.lock().await.personal_snapshot(token)
        }
    } else {
        None
    };
    let mut response = match snapshot {
        Some(snapshot) => axum::Json(snapshot).into_response(),
        None => (
            StatusCode::NOT_FOUND,
            "Este cartón ya no está disponible. Usa !carton en el chat.",
        )
            .into_response(),
    };
    response
        .headers_mut()
        .insert("cache-control", "no-store".parse().unwrap());
    response
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn personal_links_are_stable_private_and_expire_between_rounds() {
        let mut game = Bingo::default();
        game.open().unwrap();
        let card = game.register("SeniorDai").unwrap();
        let token = game.card_tokens["seniordai"].clone();
        assert_eq!(
            game.card_link("SeniorDai", "https://example.com/", "alpha"),
            format!("https://example.com/bingo.html#ch=alpha&token={token}")
        );
        assert_eq!(
            game.personal_snapshot(&token).unwrap()["card"],
            serde_json::json!(card)
        );
        assert!(game.personal_snapshot("invalid").is_none());
        assert!(!game.snapshot().to_string().contains(&token));
        game.register("SENIORDAI").unwrap();
        assert_eq!(game.card_tokens["seniordai"], token);
        game.register("other").unwrap();
        assert_ne!(game.card_tokens["other"], token);
        game.command("owner", "cancelar", true).unwrap();
        assert!(game.personal_snapshot(&token).is_none());
        game.open().unwrap();
        game.register("SeniorDai").unwrap();
        assert_ne!(game.card_tokens["seniordai"], token);
        assert!(game.personal_snapshot(&token).is_none());
    }

    #[test]
    fn repeated_card_command_displays_the_same_registered_card_during_play() {
        let mut game = Bingo::default();
        game.open().unwrap();
        let (original, _) = game.command("viewer", "carton", false).unwrap();
        let card = game.cards["viewer"];
        game.start().unwrap();
        let (repeated, _) = game.command("viewer", "carton", false).unwrap();
        assert_eq!(repeated, original);
        assert_eq!(game.cards["viewer"], card);
        assert_eq!(game.cards.len(), 1);
    }

    #[tokio::test]
    async fn personal_card_endpoint_limits_access_and_validates_real_draws() {
        use axum::{
            extract::{Path, State},
            http::{HeaderMap, StatusCode},
        };
        let pool = sqlx::postgres::PgPoolOptions::new()
            .connect_lazy("postgres://test:test@127.0.0.1/unused")
            .unwrap();
        let app = crate::test_support::app(pool);
        let (channel, _) = crate::test_support::channel("alpha", 1);
        app.channels.insert("alpha".into(), channel.clone());
        let token;
        {
            let mut game = channel.bingo.lock().await;
            game.open().unwrap();
            let card = game.register("viewer").unwrap();
            token = game.card_tokens["viewer"].clone();
            game.start().unwrap();
            assert_eq!(game.personal_snapshot(&token).unwrap()["ready"], false);
            game.drawn = card[..5].to_vec();
            assert_eq!(game.personal_snapshot(&token).unwrap()["ready"], true);
            assert!(game.claim("other").is_err());
        }
        let mut headers = HeaderMap::new();
        let missing =
            personal_card(State(app.clone()), Path("alpha".into()), headers.clone()).await;
        assert_eq!(missing.status(), StatusCode::NOT_FOUND);
        headers.insert("x-bingo-token", token.parse().unwrap());
        let response =
            personal_card(State(app.clone()), Path("alpha".into()), headers.clone()).await;
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(response.headers()["cache-control"], "no-store");
        let bytes = axum::body::to_bytes(response.into_body(), 8192)
            .await
            .unwrap();
        let body: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(body["user"], "viewer");
        assert_eq!(body["ready"], true);
        let wrong_channel =
            personal_card(State(app.clone()), Path("other".into()), headers.clone()).await;
        assert_eq!(wrong_channel.status(), StatusCode::NOT_FOUND);
        channel
            .bingo
            .lock()
            .await
            .command("owner", "cancelar", true)
            .unwrap();
        let expired =
            personal_card(State(app.clone()), Path("alpha".into()), headers.clone()).await;
        assert_eq!(expired.status(), StatusCode::NOT_FOUND);
        assert!(channel.bingo.lock().await.cards.is_empty());
    }

    #[tokio::test]
    async fn chat_dispatch_starts_automatic_draw_and_cancel_clears_the_game() {
        let pool = sqlx::postgres::PgPoolOptions::new()
            .connect_lazy("postgres://unused:unused@127.0.0.1/unused")
            .unwrap();
        let app = crate::test_support::app(pool);
        let (ch, _rx) = crate::test_support::channel("alpha", 1);
        *ch.channel_id.write().await = None;
        crate::commands::handle("intruder", "!ruleta", false, &ch, &app).await;
        assert_eq!(ch.bingo.lock().await.snapshot()["phase"], "idle");
        crate::commands::handle("owner", "!ruleta", true, &ch, &app).await;
        crate::commands::handle("viewer", "!carton", false, &ch, &app).await;
        assert_eq!(ch.bingo.lock().await.cards.len(), 1);
        let started = tokio::time::Instant::now();
        crate::commands::handle("owner", "!bingo iniciar", true, &ch, &app).await;
        assert!(ch.bingo.lock().await.drawn.is_empty());
        tokio::time::sleep(Duration::from_millis(100)).await;
        assert!(ch.bingo.lock().await.drawn.is_empty());
        tokio::time::timeout(Duration::from_secs(12), async {
            loop {
                if !ch.bingo.lock().await.drawn.is_empty() {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(25)).await;
            }
        })
        .await
        .unwrap();
        assert!(started.elapsed() >= Duration::from_secs(10));
        assert_eq!(ch.bingo.lock().await.drawn.len(), 1);
        crate::commands::handle("owner", "!bingo cancelar", true, &ch, &app).await;
        let game = ch.bingo.lock().await;
        assert_eq!(game.snapshot()["phase"], "idle");
        assert!(game.drawn.is_empty());
        assert!(game.cards.is_empty());
        drop(game);
        ch.cancel.cancel();
    }

    #[test]
    fn commands_enforce_owner_controls_cancel_tasks_and_announce_only_one_winner() {
        let mut game = Bingo::default();
        assert!(game.command("viewer", "abrir", false).is_err());
        assert!(game.command("viewer", "cancelar", false).is_err());
        game.command("owner", "abrir", true).unwrap();
        game.command("viewer", "carton", false).unwrap();
        assert!(game.command("viewer", "iniciar", false).is_err());
        assert!(game.command("owner", "ganador viewer", true).is_err());
        let (_, round) = game.command("owner", "iniciar", true).unwrap();
        assert!(game.command("owner", "iniciar", true).is_err());
        let card = game.cards["viewer"];
        game.drawn = card[..5].to_vec();
        let (message, task) = game.command("VIEWER", "", false).unwrap();
        assert!(message.contains("VIEWER gana"));
        assert!(task.is_none());
        assert_eq!(game.snapshot()["card"], serde_json::json!(card));
        assert!(game.command("viewer", "", false).is_err());
        assert!(game.draw(round.unwrap()).is_none());
        game.command("owner", "cancelar", true).unwrap();
        assert_eq!(game.snapshot()["phase"], "idle");
        assert!(game.cards.is_empty());
        game.command("owner", "abrir", true).unwrap();
        game.command("viewer", "carton", false).unwrap();
        game.command("owner", "iniciar", true).unwrap();
        assert!(game.draw(round.unwrap()).is_none());
    }

    #[test]
    fn cards_have_correct_columns_unique_numbers_and_free_center() {
        for _ in 0..100 {
            let card = generate_card();
            assert_eq!(card[12], 0);
            let mut seen = std::collections::HashSet::new();
            for (i, &n) in card.iter().enumerate() {
                if i == 12 {
                    continue;
                }
                let col = i % 5;
                assert!((col as u8 * 15 + 1..=col as u8 * 15 + 15).contains(&n));
                assert!(seen.insert(n));
            }
        }
    }

    #[test]
    fn lifecycle_closes_registration_draws_without_repeats_and_rejects_old_tasks() {
        let mut game = Bingo::default();
        assert!(game.start().is_err());
        game.open().unwrap();
        let card = game.register("Viewer").unwrap();
        assert_eq!(card, game.register("VIEWER").unwrap());
        let round = game.start().unwrap();
        assert!(game.register("late").is_err());
        assert!(game.open().is_err());
        assert!(game.draw(round + 1).is_none());
        for _ in 0..75 {
            game.draw(round).unwrap();
        }
        assert!(game.draw(round).is_none());
        let unique: std::collections::HashSet<_> = game.drawn.iter().collect();
        assert_eq!(unique.len(), 75);
        game.claim("VIEWER").unwrap();
        assert!(game.claim("Viewer").is_err());
        game.open().unwrap();
        assert!(game.draw(round).is_none());
        assert!(game.cards.is_empty());
    }

    #[test]
    fn validates_every_line_and_rejects_missing_numbers_and_unregistered_claims() {
        let card = generate_card();
        let mut lines: Vec<Vec<usize>> = (0..5)
            .map(|r| (0..5).map(|c| r * 5 + c).collect())
            .collect();
        lines.extend((0..5).map(|c| (0..5).map(|r| r * 5 + c).collect::<Vec<_>>()));
        lines.extend([vec![0, 6, 12, 18, 24], vec![4, 8, 12, 16, 20]]);
        for line in lines {
            let mut drawn: Vec<u8> = line.iter().map(|&i| card[i]).filter(|&n| n != 0).collect();
            assert!(winning_line(&card, &drawn).is_some());
            drawn.pop();
            assert!(winning_line(&card, &drawn).is_none());
        }
        let mut game = Bingo::default();
        game.open().unwrap();
        game.register("a").unwrap();
        game.start().unwrap();
        assert!(game.claim("a").is_err());
        assert!(game.claim("unknown").is_err());
        assert_eq!(game.phase, "running");
    }
}
