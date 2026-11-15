//! Convierte los eventos de chat de Kick (webhook ya verificado) al formato común.

use crate::platform::{ChatMessage, Platform, Role};

/// Moderadores de Kick a los que se les retiran los permisos de owner aunque tengan la insignia.
const REVOKED_MODERATORS: &[&str] = &["theblastv2"];

/// Mensaje de `chat.message.sent`; `None` si falta el remitente o el texto está vacío.
pub fn chat_message(json: &serde_json::Value, broadcaster_id: u64) -> Option<ChatMessage> {
    let sender = &json["sender"];
    let username = sender["username"].as_str()?;
    let text = json["content"]
        .as_str()
        .map(str::trim)
        .filter(|s| !s.is_empty())?;
    Some(ChatMessage {
        username: username.to_owned(),
        text: text.to_owned(),
        role: role(sender, broadcaster_id),
        color: sender["identity"]["username_color"]
            .as_str()
            .unwrap_or("")
            .to_owned(),
        badges: badges(sender)
            .filter_map(|b| b["type"].as_str().map(str::to_owned))
            .collect(),
    })
}

/// Rol de un usuario de Kick (remitente de un mensaje o autor de un follow). El webhook está
/// firmado, así que ni el ID, ni el nombre, ni las insignias se pueden suplantar. Las insignias
/// llegan con cada mensaje: quitar el rol en Kick revoca el acceso.
pub fn role(user: &serde_json::Value, broadcaster_id: u64) -> Role {
    let name = user["username"].as_str().map(str::trim);
    if user["user_id"].as_u64() == Some(broadcaster_id) {
        Role::Broadcaster
    } else if name.is_some_and(|n| n.eq_ignore_ascii_case(Platform::Kick.full_access_login())) {
        Role::FullAccess
    } else if !name.is_some_and(|n| REVOKED_MODERATORS.iter().any(|r| n.eq_ignore_ascii_case(r)))
        && badges(user).any(|b| b["type"] == "moderator")
    {
        Role::Moderator
    } else {
        Role::Viewer
    }
}

fn badges(user: &serde_json::Value) -> impl Iterator<Item = &serde_json::Value> {
    user["identity"]["badges"].as_array().into_iter().flatten()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn owner(user: serde_json::Value) -> bool {
        role(&user, 7).has_owner_access()
    }

    #[test]
    fn broadcaster_and_seniordai_have_owner_access_on_any_channel() {
        let sender = |name: &str, id: u64| serde_json::json!({"username": name, "user_id": id});
        assert_eq!(role(&sender("Frank_Arr", 7), 7), Role::Broadcaster);
        for name in ["seniordai", "SeniorDai", "SENIORDAI"] {
            assert_eq!(role(&sender(name, 99), 7), Role::FullAccess);
        }
        assert!(!owner(sender("viewer", 99)));
        assert!(!owner(sender("seniordai2", 99)));
        assert!(!owner(sender("senior-dai", 99)));
        assert!(!owner(serde_json::json!({})));
    }

    #[test]
    fn channel_moderators_have_owner_access() {
        let sender = |badges: serde_json::Value| serde_json::json!({"username": "mod", "user_id": 99, "identity": {"badges": badges}});
        assert_eq!(
            role(
                &sender(serde_json::json!([
                    {"text": "Subscriber", "type": "subscriber", "count": 3},
                    {"text": "Moderator", "type": "moderator"}
                ])),
                7
            ),
            Role::Moderator
        );
        for badge in ["vip", "subscriber", "og", "founder", "sub_gifter"] {
            assert!(!owner(sender(
                serde_json::json!([{"text": "Moderator", "type": badge}])
            )));
        }
        assert!(!owner(sender(serde_json::json!([]))));
        assert!(!owner(sender(serde_json::json!(null))));
        assert!(!owner(
            serde_json::json!({"username": "mod", "user_id": 99, "identity": {}})
        ));
    }

    #[test]
    fn revoked_moderators_lose_owner_access() {
        let moderator = serde_json::json!([{"text": "Moderator", "type": "moderator"}]);
        for name in ["TheBlastv2", "theblastv2", " THEBLASTV2 "] {
            assert!(!owner(
                serde_json::json!({"username": name, "user_id": 99, "identity": {"badges": moderator}})
            ));
        }
        for name in ["theblastv22", "theblast", "otro_mod"] {
            assert!(owner(
                serde_json::json!({"username": name, "user_id": 99, "identity": {"badges": moderator}})
            ));
        }
    }

    #[test]
    fn chat_messages_keep_text_color_and_badges() {
        let json = serde_json::json!({
            "sender": {"user_id": 99, "username": "Ana", "identity": {
                "username_color": "#FF00AA",
                "badges": [{"type": "subscriber"}, {"type": "moderator"}]
            }},
            "content": "  !play hola  ",
        });
        let msg = chat_message(&json, 7).unwrap();
        assert_eq!(msg.username, "Ana");
        assert_eq!(msg.text, "!play hola");
        assert_eq!(msg.role, Role::Moderator);
        assert_eq!(msg.color, "#FF00AA");
        assert_eq!(msg.badges, ["subscriber", "moderator"]);
        assert!(chat_message(
            &serde_json::json!({"sender": {"username": "Ana"}, "content": "  "}),
            7
        )
        .is_none());
        assert!(chat_message(&serde_json::json!({"content": "hola"}), 7).is_none());
    }
}
