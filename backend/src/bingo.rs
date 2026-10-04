use crate::{
    kick::sender,
    server::ns_emit,
    state::{AppState, ChannelState},
};
use rand::seq::SliceRandom;
use std::{collections::HashMap, sync::Arc, time::Duration};

pub type Card = [u8; 25];

fn card_message(user: &str, card: &Card) -> String {
    let letters = ['B', 'I', 'N', 'G', 'O'];
    let rows: Vec<String> = card
        .chunks(5)
        .enumerate()
        .map(|(index, row)| {
            let cells: Vec<String> = row
                .iter()
                .enumerate()
                .map(|(col, number)| {
                    if *number == 0 {
                        "LIBRE".to_string()
                    } else {
                        format!("{}{number}", letters[col])
                    }
                })
                .collect();
            format!("Fila {} [{}]", index + 1, cells.join(" "))
        })
        .collect();
    format!(
        "{user}, tu cartón: {}. Marca las bolas que salgan; LIBRE ya cuenta. Ganas con una fila, columna o diagonal completa. Escribe !bingo y el bot lo verifica. Repite !carton para consultar el mismo cartón.",
        rows.join(" | ")
    )
}

#[derive(Default)]
pub struct Bingo {
    round: u64,
    phase: &'static str,
    cards: HashMap<String, Card>,
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
        self.cards.insert(key, card);
        Ok(card)
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
                let card = self.register(user)?;
                Ok((card_message(user, &card), None))
            }
            "iniciar" => {
                let round = self.start()?;
                Ok(("Bingo iniciado. Inscripciones cerradas; una bola cada 10 segundos. Reclama tu línea con !bingo.".into(), Some(round)))
            }
            "cancelar" => {
                self.phase = "idle";
                self.round += 1;
                self.cards.clear();
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
    let result = game.command(user, args, owner);
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn card_message_labels_rows_numbers_free_center_and_claim_instructions() {
        let card = [
            12, 20, 36, 56, 61, 1, 19, 40, 53, 71, 7, 22, 0, 51, 65, 4, 29, 43, 58, 74, 8, 17, 32,
            46, 66,
        ];
        assert_eq!(
            card_message("SeniorDai", &card),
            "SeniorDai, tu cartón: Fila 1 [B12 I20 N36 G56 O61] | Fila 2 [B1 I19 N40 G53 O71] | Fila 3 [B7 I22 LIBRE G51 O65] | Fila 4 [B4 I29 N43 G58 O74] | Fila 5 [B8 I17 N32 G46 O66]. Marca las bolas que salgan; LIBRE ya cuenta. Ganas con una fila, columna o diagonal completa. Escribe !bingo y el bot lo verifica. Repite !carton para consultar el mismo cartón."
        );
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
