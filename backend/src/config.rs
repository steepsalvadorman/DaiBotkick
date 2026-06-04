use std::env;

#[derive(Clone, Debug)]
pub struct Config {
    pub channel_name: String,
    // OAuth 2.0 (preferido)
    pub access_token: String,
    pub refresh_token: String,
    pub client_id: String,
    pub client_secret: String,
    pub token_expires: u64,  // unix timestamp
    // Cookies de sesión (fallback si no hay OAuth)
    pub cookies: String,
    pub bearer_token: String,
    pub xsrf_token: String,
    pub panel_token: String,
    // Respuestas configurables de comandos del chat
    pub cmd_discord: String,
    pub cmd_redes:   String,
    pub cmd_pc:      String,
    pub cmd_horario: String,
    pub follow_goal: u64,
    pub current_followers: u64,
    pub port: u16,
    pub overlay_dir: String,
    pub queue_file: String,
    pub tts_cache_dir: String,
}

impl Config {
    pub fn from_env() -> Result<Self, String> {
        let access_token  = env::var("KICK_ACCESS_TOKEN").unwrap_or_default();
        let refresh_token = env::var("KICK_REFRESH_TOKEN").unwrap_or_default();

        let cookies = env::var("COOKIES").unwrap_or_default();
        if access_token.is_empty() && cookies.is_empty() {
            return Err("no_auth".to_string());
        }

        let bearer_token = env::var("BEARER_TOKEN").unwrap_or_else(|_| {
            extract_cookie(&cookies, "kick_session").unwrap_or_default()
        });
        let xsrf_token = env::var("XSRF_TOKEN").unwrap_or_else(|_| {
            extract_cookie(&cookies, "XSRF-TOKEN").unwrap_or_default()
        });

        Ok(Self {
            channel_name: env::var("CHANNEL_NAME").unwrap_or_default(),
            access_token,
            refresh_token,
            client_id: env::var("KICK_CLIENT_ID").unwrap_or_default(),
            client_secret: env::var("KICK_CLIENT_SECRET").unwrap_or_default(),
            token_expires: env::var("KICK_TOKEN_EXPIRES")
                .ok().and_then(|v| v.trim_matches('"').parse().ok()).unwrap_or(0),
            cookies,
            bearer_token: url_decode(&bearer_token),
            xsrf_token: url_decode(&xsrf_token),
            panel_token: env::var("PANEL_TOKEN").unwrap_or_default(),
            cmd_discord: env::var("CMD_DISCORD").unwrap_or_default(),
            cmd_redes:   env::var("CMD_REDES").unwrap_or_default(),
            cmd_pc:      env::var("CMD_PC").unwrap_or_default(),
            cmd_horario: env::var("CMD_HORARIO").unwrap_or_default(),
            follow_goal: env_u64("FOLLOW_GOAL", 100),
            current_followers: env_u64("CURRENT_FOLLOWERS", 0),
            port: env::var("PORT").ok().and_then(|v| v.parse().ok()).unwrap_or(3000),
            overlay_dir: env::var("OVERLAY_DIR").unwrap_or_else(|_| "../overlay".into()),
            queue_file: env::var("QUEUE_FILE").unwrap_or_else(|_| "../data/queue.json".into()),
            tts_cache_dir: env::var("TTS_CACHE_DIR").unwrap_or_else(|_| "../data/tts_cache".into()),
        })
    }
}

fn extract_cookie(cookies: &str, key: &str) -> Option<String> {
    cookies.split(';').find_map(|part| {
        part.trim().strip_prefix(&format!("{key}=")).map(|v| v.to_string())
    })
}

fn url_decode(s: &str) -> String {
    url::form_urlencoded::parse(s.replace('+', " ").as_bytes())
        .map(|(k, _)| k.into_owned())
        .next()
        .unwrap_or_else(|| s.to_string())
}

fn env_u64(key: &str, default: u64) -> u64 {
    env::var(key).ok().and_then(|v| v.parse().ok()).unwrap_or(default)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extract_cookie_finds_value() {
        let cookies = "kick_session=abc123; XSRF-TOKEN=xyz789; other=val";
        assert_eq!(extract_cookie(cookies, "kick_session"), Some("abc123".into()));
        assert_eq!(extract_cookie(cookies, "XSRF-TOKEN"),   Some("xyz789".into()));
        assert_eq!(extract_cookie(cookies, "other"),        Some("val".into()));
    }

    #[test]
    fn extract_cookie_returns_none_when_missing() {
        let cookies = "kick_session=abc123";
        assert_eq!(extract_cookie(cookies, "nonexistent"), None);
    }

    #[test]
    fn extract_cookie_empty_string() {
        assert_eq!(extract_cookie("", "kick_session"), None);
    }

    #[test]
    fn extract_cookie_no_whitespace_ambiguity() {
        // El parser hace trim() en cada part, así que con espacios funciona igual
        let cookies = "  kick_session=abc123  ;  other=val  ";
        assert_eq!(extract_cookie(cookies, "kick_session"), Some("abc123".into()));
    }

    #[test]
    fn url_decode_plain_ascii_unchanged() {
        assert_eq!(url_decode("hello"), "hello");
        assert_eq!(url_decode("seniordai"), "seniordai");
    }

    #[test]
    fn url_decode_percent_encoded_space() {
        assert_eq!(url_decode("hello%20world"), "hello world");
    }

    #[test]
    fn url_decode_plus_as_space() {
        assert_eq!(url_decode("hello+world"), "hello world");
    }
}
