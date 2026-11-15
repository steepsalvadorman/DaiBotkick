//! Convierte los eventos EventSub de Twitch (ya verificados y guardados en el inbox) al
//! formato común y los entrega a chat, comandos y alertas.

use crate::{
    chat,
    platform::{ChatMessage, Platform, Role},
    state::AppState,
};
use std::sync::Arc;

/// ID de Twitch de `sr_dai_twitch`: acceso total en todos los canales de Twitch.
pub const FULL_ACCESS_USER_ID: &str = "701010126";

pub async fn process(app: &Arc<AppState>, kind: &str, json: &serde_json::Value) {
    let event = &json["event"];
    let Some(broadcaster) = event["broadcaster_user_id"]
        .as_str()
        .and_then(|id| id.parse::<u64>().ok())
    else {
        return;
    };
    let channel = app
        .user_id_to_key
        .get(&(Platform::Twitch, broadcaster))
        .and_then(|key| app.channels.get(key.value()).map(|c| c.clone()));
    let Some(ch) = channel else {
        return;
    };
    match kind {
        "channel.chat.message" => {
            // DaiBotsito no se responde a sí mismo ni aparece dos veces en el chat.
            let bot = app
                .twitch_bot
                .read()
                .await
                .as_ref()
                .map(|b| b.user_id.clone());
            if event["chatter_user_id"]
                .as_str()
                .is_some_and(|id| Some(id) == bot.as_deref())
            {
                return;
            }
            if let Some(msg) = chat_message(event) {
                chat::receive(app, &ch, &msg).await;
            }
        }
        "channel.follow" => {
            let role = if event["user_id"] == event["broadcaster_user_id"] {
                Role::Broadcaster
            } else if event["user_id"] == FULL_ACCESS_USER_ID {
                Role::FullAccess
            } else {
                Role::Viewer
            };
            chat::followed(app, &ch, &display_name(event, "user"), role).await;
        }
        // Los regalos llegan también como `channel.subscription.gift`, que ya anuncia.
        "channel.subscribe" if event["is_gift"] != true => {
            chat::subscribed(app, &ch, &display_name(event, "user"), 1);
        }
        "channel.subscription.message" => {
            let months = event["cumulative_months"].as_u64().unwrap_or(1).max(1);
            chat::subscribed(app, &ch, &display_name(event, "user"), months);
        }
        "channel.subscription.gift" => {
            let gifter = if event["is_anonymous"] == true {
                "Un anónimo".to_owned()
            } else {
                display_name(event, "user")
            };
            chat::gifted(app, &ch, &gifter, event["total"].as_u64().unwrap_or(1));
        }
        "stream.online" => {
            chat::live_status(app, &ch, chat::parse_time(&event["started_at"]), true).await;
        }
        "stream.offline" => chat::live_status(app, &ch, None, false).await,
        _ => tracing::debug!("[Twitch] Ignorando tipo {kind}"),
    }
}

/// Mensaje de `channel.chat.message`; `None` si falta el autor o el texto está vacío.
pub fn chat_message(event: &serde_json::Value) -> Option<ChatMessage> {
    event["chatter_user_login"].as_str()?;
    let text = event["message"]["text"]
        .as_str()
        .map(str::trim)
        .filter(|t| !t.is_empty())?;
    let badges: Vec<String> = event["badges"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|b| b["set_id"].as_str().map(str::to_owned))
        .collect();
    let role = if event["chatter_user_id"] == event["broadcaster_user_id"] {
        Role::Broadcaster
    } else if event["chatter_user_id"] == FULL_ACCESS_USER_ID {
        Role::FullAccess
    } else if badges.iter().any(|b| b == "moderator") {
        Role::Moderator
    } else {
        Role::Viewer
    };
    Some(ChatMessage {
        username: display_name(event, "chatter_user"),
        text: text.to_owned(),
        role,
        color: event["color"]
            .as_str()
            .filter(|c| c.len() == 7 && c.starts_with('#'))
            .unwrap_or("")
            .to_owned(),
        badges,
    })
}

/// Nombre visible si solo cambia mayúsculas respecto al login (`Sr_Dai` / `sr_dai`); si no
/// (nombres en otros alfabetos), el login, que es el que sirve para `!ban` y las menciones.
fn display_name(event: &serde_json::Value, prefix: &str) -> String {
    let login = event[format!("{prefix}_login")]
        .as_str()
        .unwrap_or("alguien");
    match event[format!("{prefix}_name")].as_str() {
        Some(name) if name.eq_ignore_ascii_case(login) => name.to_owned(),
        _ => login.to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn message(
        chatter_id: &str,
        login: &str,
        name: &str,
        badges: serde_json::Value,
    ) -> serde_json::Value {
        serde_json::json!({
            "broadcaster_user_id": "5",
            "chatter_user_id": chatter_id,
            "chatter_user_login": login,
            "chatter_user_name": name,
            "message": {"text": "  !play hola  ", "fragments": []},
            "color": "#FF4500",
            "badges": badges,
        })
    }

    #[test]
    fn chat_roles_come_from_ids_and_the_moderator_badge() {
        let none = serde_json::json!([]);
        let role = |id: &str, badges: &serde_json::Value| {
            chat_message(&message(id, "ana", "Ana", badges.clone()))
                .unwrap()
                .role
        };
        assert_eq!(role("5", &none), Role::Broadcaster);
        assert_eq!(role(FULL_ACCESS_USER_ID, &none), Role::FullAccess);
        let moderator = serde_json::json!([{"set_id": "subscriber", "id": "12"}, {"set_id": "moderator", "id": "1"}]);
        assert_eq!(role("9", &moderator), Role::Moderator);
        for badge in ["vip", "subscriber", "broadcaster"] {
            assert_eq!(
                role("9", &serde_json::json!([{"set_id": badge}])),
                Role::Viewer,
                "{badge}"
            );
        }
        assert_eq!(role("9", &none), Role::Viewer);
    }

    #[test]
    fn chat_messages_keep_text_color_badges_and_a_usable_name() {
        let msg = chat_message(&message(
            "9",
            "ana_7",
            "Ana_7",
            serde_json::json!([{"set_id": "vip"}]),
        ))
        .unwrap();
        assert_eq!(msg.username, "Ana_7");
        assert_eq!(msg.text, "!play hola");
        assert_eq!(msg.color, "#FF4500");
        assert_eq!(msg.badges, ["vip"]);
        // Un nombre visible en otro alfabeto no sirve para `!ban`: se usa el login.
        let japanese =
            chat_message(&message("9", "tanaka", "田中", serde_json::json!([]))).unwrap();
        assert_eq!(japanese.username, "tanaka");
        let mut empty = message("9", "ana", "Ana", serde_json::json!([]));
        empty["message"]["text"] = "   ".into();
        assert!(chat_message(&empty).is_none());
        let mut bad_color = message("9", "ana", "Ana", serde_json::json!([]));
        bad_color["color"] = "red;}".into();
        assert_eq!(chat_message(&bad_color).unwrap().color, "");
    }
}
