//! Tipos comunes a todas las plataformas de streaming. Los comandos, el bingo y el overlay
//! trabajan con estos tipos; cada plataforma (`kick`, y más adelante `twitch`) convierte sus
//! eventos a `ChatMessage` y recibe las respuestas mediante `crate::chat::send`.

/// Plataforma a la que pertenece un canal de DaiBot. Un canal vive en una sola plataforma.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Platform {
    #[default]
    Kick,
    /// Se construye cuando los canales de Twitch se cargan de la base (etapa 2 del plan).
    #[allow(dead_code)]
    Twitch,
}

impl Platform {
    /// Cuenta con acceso total a todos los canales de esa plataforma (sin cooldowns ni baneos).
    pub fn full_access_login(self) -> &'static str {
        match self {
            Platform::Kick => "seniordai",
            Platform::Twitch => "sr_dai_twitch",
        }
    }
}

/// Rol del autor de un mensaje dentro del canal, ya verificado por la plataforma.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Role {
    Broadcaster,
    /// La cuenta de `Platform::full_access_login`.
    FullAccess,
    Moderator,
    Viewer,
}

impl Role {
    /// Streamer, moderadores y la cuenta de acceso total usan todos los comandos sin cooldowns.
    pub fn has_owner_access(self) -> bool {
        self != Role::Viewer
    }

    /// El streamer y la cuenta de acceso total nunca quedan bloqueados por `!ban`.
    pub fn ban_exempt(self) -> bool {
        matches!(self, Role::Broadcaster | Role::FullAccess)
    }
}

/// Mensaje de chat recibido de cualquier plataforma, con el remitente ya verificado.
#[derive(Clone, Debug)]
pub struct ChatMessage {
    pub username: String,
    /// Texto sin espacios al inicio ni al final; nunca vacío.
    pub text: String,
    pub role: Role,
    /// Color del nombre en el chat de la plataforma, si lo envía (`#RRGGBB`).
    pub color: String,
    /// Tipos de insignia tal como los nombra la plataforma (`moderator`, `subscriber`…).
    pub badges: Vec<String>,
}

#[cfg(test)]
impl ChatMessage {
    /// Mensaje mínimo para pruebas de comandos: dueño (`Broadcaster`) o espectador.
    pub fn for_test(username: &str, text: &str, is_owner: bool) -> Self {
        Self {
            username: username.into(),
            text: text.into(),
            role: if is_owner {
                Role::Broadcaster
            } else {
                Role::Viewer
            },
            color: String::new(),
            badges: Vec::new(),
        }
    }
}

/// `true` si `!ban` bloquea a este usuario en el canal. Un nombre inválido nunca está baneado.
pub async fn is_blocked(ch: &crate::state::ChannelState, username: &str, role: Role) -> bool {
    if role.ban_exempt() {
        return false;
    }
    match crate::state::normalize_username(username) {
        Some(name) => name != ch.platform.full_access_login() && ch.is_banned(&name).await,
        None => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn owner_access_and_ban_exemption_follow_the_role() {
        for role in [Role::Broadcaster, Role::FullAccess, Role::Moderator] {
            assert!(role.has_owner_access());
        }
        assert!(!Role::Viewer.has_owner_access());
        assert!(Role::Broadcaster.ban_exempt() && Role::FullAccess.ban_exempt());
        assert!(!Role::Moderator.ban_exempt() && !Role::Viewer.ban_exempt());
    }

    #[test]
    fn each_platform_has_its_own_full_access_account() {
        assert_eq!(Platform::Kick.full_access_login(), "seniordai");
        assert_eq!(Platform::Twitch.full_access_login(), "sr_dai_twitch");
    }
}
