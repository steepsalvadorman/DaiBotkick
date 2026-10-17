// Videos de TikTok: se reproducen con el reproductor embebido oficial
// (https://www.tiktok.com/player/v1/{id}), sin descargar el video.
use std::time::Duration;

const BROWSER_UA: &str =
    "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/141.0 Safari/537.36";

fn is_tiktok_host(host: &str) -> bool {
    host == "tiktok.com" || host.ends_with(".tiktok.com")
}

/// Indica si el enlace apunta a TikTok, aunque todavía no se conozca el ID.
pub fn is_link(value: &str) -> bool {
    url::Url::parse(value)
        .ok()
        .filter(|url| url.scheme() == "https")
        .and_then(|url| url.host_str().map(is_tiktok_host))
        .unwrap_or(false)
}

/// Extrae el ID numérico de un enlace completo de TikTok.
pub fn id(value: &str) -> Option<String> {
    let url = url::Url::parse(value).ok()?;
    if url.scheme() != "https"
        || !matches!(
            url.host_str()?,
            "tiktok.com" | "www.tiktok.com" | "m.tiktok.com"
        )
    {
        return None;
    }
    let segments: Vec<&str> = url.path_segments()?.filter(|s| !s.is_empty()).collect();
    let candidate = match segments.as_slice() {
        // /@usuario/video/{id}; los enlaces cortos redirigen a veces a /@/video/{id}.
        [user, "video", id] if user.starts_with('@') => (*id).to_owned(),
        ["video", id] | ["embed", id] | ["embed", "v2", id] | ["player", "v1", id] => {
            (*id).to_owned()
        }
        ["v", file] => file.strip_suffix(".html")?.to_owned(),
        _ => url
            .query_pairs()
            .find(|(k, _)| k == "share_item_id")?
            .1
            .into_owned(),
    };
    valid_id(&candidate).then_some(candidate)
}

fn valid_id(value: &str) -> bool {
    (15..=25).contains(&value.len()) && value.bytes().all(|b| b.is_ascii_digit())
}

/// Resuelve enlaces cortos (vm.tiktok.com, vt.tiktok.com, tiktok.com/t/...) siguiendo
/// redirecciones solo dentro de dominios de TikTok, y obtiene el título por oEmbed.
pub async fn resolve(value: &str) -> Result<(String, String), &'static str> {
    let client = reqwest::Client::builder()
        .user_agent(BROWSER_UA)
        .timeout(Duration::from_secs(15))
        .redirect(reqwest::redirect::Policy::custom(|attempt| {
            let allowed = attempt.url().scheme() == "https"
                && attempt.url().host_str().is_some_and(is_tiktok_host);
            if !allowed || attempt.previous().len() >= 5 || id(attempt.url().as_str()).is_some() {
                attempt.stop()
            } else {
                attempt.follow()
            }
        }))
        .build()
        .map_err(|_| "No se pudo preparar la consulta a TikTok.")?;

    let canonical = match id(value) {
        Some(_) => value.to_owned(),
        None => {
            let response =
                client.get(value).send().await.map_err(|_| {
                    "TikTok no respondió. Intenta con el enlace completo del video."
                })?;
            let target = response
                .headers()
                .get(reqwest::header::LOCATION)
                .and_then(|v| v.to_str().ok())
                .and_then(|location| response.url().join(location).ok())
                .unwrap_or_else(|| response.url().clone());
            target.to_string()
        }
    };
    let video_id = id(&canonical)
        .ok_or("No encontré un video en ese enlace de TikTok. Usa el enlace completo del video.")?;
    Ok((
        video_id.clone(),
        title(&client, &video_id, &canonical).await,
    ))
}

async fn title(client: &reqwest::Client, video_id: &str, canonical: &str) -> String {
    let fallback = || "Video de TikTok".to_string();
    let mut page = url::Url::parse(canonical).ok();
    if let Some(page) = page.as_mut() {
        page.set_query(None);
    }
    let page = page
        .filter(|p| !p.path().starts_with("/@/"))
        .map(|p| p.to_string())
        .unwrap_or_else(|| format!("https://www.tiktok.com/@_/video/{video_id}"));
    let Ok(api) = url::Url::parse_with_params("https://www.tiktok.com/oembed", [("url", page)])
    else {
        return fallback();
    };
    let Ok(response) = client.get(api).send().await else {
        return fallback();
    };
    let Ok(json) = response.json::<serde_json::Value>().await else {
        return fallback();
    };
    let text = json["title"].as_str().unwrap_or("").trim();
    let author = json["author_name"].as_str().unwrap_or("").trim();
    match (text.is_empty(), author.is_empty()) {
        (false, _) => super::trunc(text, 200).to_owned(),
        (true, false) => format!("TikTok de {}", super::trunc(author, 60)),
        (true, true) => fallback(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_ids_from_full_links_only_on_tiktok_hosts() {
        let id_value = "7440114099565972791";
        for link in [
            "https://www.tiktok.com/@seniordai/video/7440114099565972791?is_from_webapp=1",
            "https://www.tiktok.com/@/video/7440114099565972791?_r=1",
            "https://m.tiktok.com/v/7440114099565972791.html",
            "https://www.tiktok.com/embed/v2/7440114099565972791",
            "https://www.tiktok.com/player/v1/7440114099565972791",
            "https://www.tiktok.com/?share_item_id=7440114099565972791",
        ] {
            assert_eq!(id(link).as_deref(), Some(id_value), "{link}");
        }
        assert!(id("http://www.tiktok.com/@a/video/7440114099565972791").is_none());
        assert!(id("https://tiktok.com.evil.example/@a/video/7440114099565972791").is_none());
        assert!(id("https://www.tiktok.com/@a/video/123").is_none());
        assert!(id("https://www.tiktok.com/@a/video/74401140995659727x1").is_none());
        assert!(id("https://vm.tiktok.com/ZMhvqjqjq/").is_none());
    }

    #[test]
    fn recognizes_short_links_as_tiktok() {
        assert!(is_link("https://vm.tiktok.com/ZMhvqjqjq/"));
        assert!(is_link("https://vt.tiktok.com/ZSabc123/"));
        assert!(is_link("https://www.tiktok.com/t/ZTRav7308/"));
        assert!(!is_link("https://tiktok.com.evil.example/t/x"));
        assert!(!is_link("http://vm.tiktok.com/ZMhvqjqjq/"));
    }

    #[tokio::test]
    #[ignore = "requires access to TikTok"]
    async fn live_short_link_resolves_to_a_video_id() {
        let (video_id, title) = resolve("https://vm.tiktok.com/ZMhvqjqjq/").await.unwrap();
        assert!(valid_id(&video_id));
        println!("{video_id}: {title}");
    }
}
