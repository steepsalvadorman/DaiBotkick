use crate::state::AppState;
use std::{sync::Arc, time::Duration};

pub async fn youtube(query: &str, app: &Arc<AppState>) -> Result<(String, String), &'static str> {
    if query.trim().is_empty() || query.chars().count() > 160 {
        return Err("Escribe el nombre y artista con un máximo de 160 caracteres.");
    }
    let _permit = app
        .search_slots
        .acquire()
        .await
        .map_err(|_| "Búsqueda no disponible.")?;
    let executable = std::env::var("YT_DLP_PATH")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|_| {
            #[cfg(windows)]
            if let Some(local) = std::env::var_os("LOCALAPPDATA") {
                let installed = std::path::PathBuf::from(local)
                    .join("DaiBot")
                    .join("tools")
                    .join("yt-dlp.exe");
                if installed.is_file() {
                    return installed;
                }
            }
            std::path::PathBuf::from("yt-dlp")
        });
    let mut command = tokio::process::Command::new(executable);
    command.args([
        "--ignore-config",
        "--no-warnings",
        "--flat-playlist",
        "--dump-single-json",
        "--skip-download",
        "--socket-timeout",
        "10",
        "--retries",
        "0",
        "--",
        &format!("ytsearch5:{query}"),
    ]);
    command.kill_on_drop(true);
    #[cfg(windows)]
    command.creation_flags(0x08000000);
    let output = tokio::time::timeout(Duration::from_secs(25), command.output())
        .await
        .map_err(|_| "YouTube tardó demasiado en responder. Intenta con un enlace.")?
        .map_err(|_| "No se pudo ejecutar yt-dlp. Instálalo en el PATH del servidor.")?;
    if !output.status.success() {
        tracing::warn!("[Search] yt-dlp failed with {}", output.status);
        return Err("YouTube bloqueó o rechazó la búsqueda. Intenta con un enlace.");
    }
    let data: serde_json::Value = serde_json::from_slice(&output.stdout)
        .map_err(|_| "La búsqueda devolvió datos inválidos.")?;
    choose(&data).ok_or(
        "No encontré una canción de hasta 10 minutos. Prueba con el nombre y artista o un enlace.",
    )
}

fn choose(data: &serde_json::Value) -> Option<(String, String)> {
    data["entries"].as_array()?.iter().find_map(|entry| {
        let id = entry["id"].as_str()?;
        if id.len() != 11
            || !id
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
        {
            return None;
        }
        let duration = entry["duration"].as_f64()?;
        if !(1.0..=600.0).contains(&duration)
            || entry["live_status"] == "is_live"
            || entry["live_status"] == "is_upcoming"
            || entry["is_live"] == true
        {
            return None;
        }
        let title = entry["title"].as_str()?.trim();
        if title.is_empty() {
            return None;
        }
        Some((id.to_owned(), super::trunc(title, 200).to_owned()))
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    #[ignore = "requires yt-dlp and access to YouTube"]
    async fn live_named_song_search_resolves_a_playable_queue_id() {
        let pool = sqlx::postgres::PgPoolOptions::new()
            .connect_lazy("postgres://unused:unused@127.0.0.1/unused")
            .unwrap();
        let app = crate::test_support::app(pool);
        let (id, title) = youtube("cervecita flor pileña", &app).await.unwrap();
        assert_eq!(id.len(), 11);
        assert!(super::super::yt_id(&format!("https://www.youtube.com/watch?v={id}")).is_some());
        assert!(!title.trim().is_empty());
        println!("Selected: {title} ({id})");
    }
    #[test]
    fn selects_first_relevant_short_video_not_live_long_or_invalid_results() {
        let data = serde_json::json!({"entries":[
            {"id":"bad", "duration":100, "title":"Bad"},
            {"id":"aaaaaaaaaaa", "duration":601, "title":"Mix"},
            {"id":"bbbbbbbbbbb", "duration":100, "title":"Live", "is_live":true},
            {"id":"ccccccccccc", "duration":100, "title":"Upcoming", "live_status":"is_upcoming"},
            {"id":"ddddddddddd", "title":"Unknown duration"},
            {"id":"eeeeeeeeeee", "duration":180, "title":"Cervecita - Flor Pileña"},
            {"id":"fffffffffff", "duration":180, "title":"Other"}
        ]});
        assert_eq!(
            choose(&data),
            Some(("eeeeeeeeeee".into(), "Cervecita - Flor Pileña".into()))
        );
        assert!(choose(&serde_json::json!({"entries":[]})).is_none());
        assert!(choose(&serde_json::json!({})).is_none());
    }
}
