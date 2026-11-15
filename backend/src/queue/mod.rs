use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VideoItem {
    pub id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub video_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    pub title: String,
    pub user: String,
}

/// Canciones ya reproducidas que se recuerdan para volver atrás.
const HISTORY_LIMIT: usize = 20;

#[derive(Debug, Default, Clone, Serialize, Deserialize)]
pub struct VideoQueue {
    pub items: Vec<VideoItem>,
    pub version: u64,
    /// Reproducidas, la más reciente al final. `default` lee colas guardadas antes de existir.
    /// Vacío no se serializa: así la cola coincide con las filas guardadas sin `history`, que
    /// `change` compara como JSON antes de escribir; si no, ninguna de esas colas se podría cambiar.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub history: Vec<VideoItem>,
}

impl VideoQueue {
    #[cfg(test)]
    pub fn new() -> Self {
        Self::default()
    }

    pub fn push(&mut self, item: VideoItem) {
        self.items.push(item);
        self.version += 1;
    }

    pub fn advance(&mut self) {
        if !self.items.is_empty() {
            let played = self.items.remove(0);
            self.history.push(played);
            if self.history.len() > HISTORY_LIMIT {
                self.history.remove(0);
            }
            self.version += 1;
        }
    }

    /// Devuelve al frente la última canción reproducida; la actual pasa a ser la siguiente.
    pub fn previous_if(&mut self, version: u64) -> bool {
        if self.version != version {
            return false;
        }
        let Some(item) = self.history.pop() else {
            return false;
        };
        self.items.insert(0, item);
        self.version += 1;
        true
    }

    /// Estado que reciben los overlays: cola, versión y si se puede volver atrás.
    pub fn sync_payload(&self) -> serde_json::Value {
        serde_json::json!({
            "items": self.items,
            "version": self.version,
            "previous": !self.history.is_empty(),
        })
    }

    pub fn remove(&mut self, index: usize) {
        if index < self.items.len() {
            self.items.remove(index);
            self.version += 1;
        }
    }

    /// Quita todas las solicitudes de `user` (sin distinguir mayúsculas); `true` si quitó alguna.
    pub fn remove_user(&mut self, user: &str) -> bool {
        let before = self.items.len();
        self.items
            .retain(|item| !item.user.eq_ignore_ascii_case(user));
        let changed = self.items.len() != before;
        if changed {
            self.version += 1;
        }
        changed
    }

    pub fn clear(&mut self) {
        self.items.clear();
        self.version += 1;
    }

    pub fn advance_if(&mut self, id: &str, version: u64) -> bool {
        if self.version != version || self.items.first().is_none_or(|i| i.id != id) {
            return false;
        }
        self.advance();
        true
    }
}

/// Serialize mutation, persist before exposing it, and emit while still holding the lock.
pub async fn change<F>(
    ch: &crate::state::ChannelState,
    global: &crate::state::AppState,
    edit: F,
) -> Result<bool, sqlx::Error>
where
    F: FnOnce(&mut VideoQueue) -> bool,
{
    let mut current = ch.video_queue.write().await;
    let mut next = current.clone();
    if !edit(&mut next) {
        return Ok(false);
    }
    let json = serde_json::to_string(&next).expect("serializable queue");
    if ch.cancel.is_cancelled() {
        return Err(sqlx::Error::Protocol("Canal detenido".into()));
    }
    let previous = serde_json::to_string(&*current).expect("serializable queue");
    let updated = sqlx::query(
        "UPDATE channels SET queue_state=$1 WHERE slug=$2 AND queue_state::jsonb=$3::text::jsonb",
    )
    .bind(json)
    .bind(&ch.slug)
    .bind(previous)
    .execute(&global.db)
    .await?;
    if updated.rows_affected() != 1 {
        let persisted: String =
            sqlx::query_scalar("SELECT queue_state FROM channels WHERE slug=$1")
                .bind(&ch.slug)
                .fetch_one(&global.db)
                .await?;
        *current =
            serde_json::from_str(&persisted).map_err(|e| sqlx::Error::Protocol(e.to_string()))?;
        crate::server::ns_emit(global, &ch.slug, "syncQueue", current.sync_payload());
        return Err(sqlx::Error::Protocol(
            "La cola cambio; vuelve a intentar".into(),
        ));
    }
    *current = next;
    crate::server::ns_emit(global, &ch.slug, "syncQueue", current.sync_payload());
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn make_queue() -> VideoQueue {
        VideoQueue::new()
    }

    /// `change` compara la cola en memoria con la fila guardada: una cola sin historial debe
    /// serializarse igual que las filas anteriores a `history` (y que el valor por defecto de la columna).
    #[test]
    fn empty_history_serializes_like_queues_saved_before_history() {
        let stored = r#"{"items":[],"version":0}"#;
        let loaded: VideoQueue = serde_json::from_str(stored).unwrap();
        assert_eq!(serde_json::to_string(&loaded).unwrap(), stored);
        let mut played = make_queue();
        played.history.push(item("a", "ana"));
        assert!(serde_json::to_string(&played)
            .unwrap()
            .contains("\"history\""));
    }

    fn item(title: &str, user: &str) -> VideoItem {
        VideoItem {
            id: uuid::Uuid::new_v4().to_string(),
            video_id: None,
            url: Some("https://youtu.be/dQw4w9WgXcQ".into()),
            title: title.into(),
            user: user.into(),
        }
    }

    #[test]
    fn starts_empty() {
        let q = make_queue();
        assert!(q.items.is_empty());
    }

    #[test]
    fn push_adds_item() {
        let mut q = make_queue();
        q.push(item("Song A", "user1"));
        assert_eq!(q.items.len(), 1);
        assert_eq!(q.items[0].title, "Song A");
        assert_eq!(q.items[0].user, "user1");
    }

    #[test]
    fn push_preserves_order() {
        let mut q = make_queue();
        q.push(item("A", "u"));
        q.push(item("B", "u"));
        q.push(item("C", "u"));
        let titles: Vec<&str> = q.items.iter().map(|v| v.title.as_str()).collect();
        assert_eq!(titles, ["A", "B", "C"]);
    }

    #[test]
    fn advance_removes_first() {
        let mut q = make_queue();
        q.push(item("A", "u"));
        q.push(item("B", "u"));
        q.advance();
        assert_eq!(q.items.len(), 1);
        assert_eq!(q.items[0].title, "B");
    }

    #[test]
    fn remove_user_drops_every_request_of_that_user_only() {
        let mut q = make_queue();
        q.push(item("A", "Troll"));
        q.push(item("B", "viewer"));
        q.push(item("C", "troll"));
        let version = q.version;
        assert!(q.remove_user("TROLL"));
        assert_eq!(
            q.items.iter().map(|i| i.title.as_str()).collect::<Vec<_>>(),
            ["B"]
        );
        assert_eq!(q.version, version + 1);
        assert!(!q.remove_user("troll"));
        assert_eq!(q.version, version + 1);
    }

    #[test]
    fn previous_returns_last_played_to_front() {
        let mut q = make_queue();
        q.push(item("A", "u"));
        q.push(item("B", "u"));
        q.advance();
        assert_eq!(q.items[0].title, "B");
        assert!(q.previous_if(q.version));
        let titles: Vec<&str> = q.items.iter().map(|v| v.title.as_str()).collect();
        assert_eq!(titles, ["A", "B"]);
        assert!(q.history.is_empty());
    }

    #[test]
    fn previous_rejects_stale_version_and_empty_history() {
        let mut q = make_queue();
        q.push(item("A", "u"));
        assert!(!q.previous_if(q.version));
        q.advance();
        assert!(!q.previous_if(q.version - 1));
        assert!(q.items.is_empty());
    }

    #[test]
    fn history_is_capped() {
        let mut q = make_queue();
        for n in 0..(HISTORY_LIMIT + 5) {
            q.push(item(&n.to_string(), "u"));
            q.advance();
        }
        assert_eq!(q.history.len(), HISTORY_LIMIT);
        assert_eq!(
            q.history.last().unwrap().title,
            (HISTORY_LIMIT + 4).to_string()
        );
    }

    #[test]
    fn queue_saved_without_history_still_loads() {
        let q: VideoQueue = serde_json::from_str(r#"{"items":[],"version":3}"#).unwrap();
        assert_eq!(q.version, 3);
        assert!(q.history.is_empty());
    }

    #[test]
    fn advance_on_empty_is_noop() {
        let mut q = make_queue();
        q.advance();
        assert!(q.items.is_empty());
    }

    #[test]
    fn remove_middle_item() {
        let mut q = make_queue();
        q.push(item("A", "u"));
        q.push(item("B", "u"));
        q.push(item("C", "u"));
        q.remove(1);
        assert_eq!(q.items.len(), 2);
        assert_eq!(q.items[0].title, "A");
        assert_eq!(q.items[1].title, "C");
    }

    #[test]
    fn remove_first_item() {
        let mut q = make_queue();
        q.push(item("A", "u"));
        q.push(item("B", "u"));
        q.remove(0);
        assert_eq!(q.items[0].title, "B");
    }

    #[test]
    fn remove_out_of_bounds_is_noop() {
        let mut q = make_queue();
        q.push(item("A", "u"));
        q.remove(5);
        assert_eq!(q.items.len(), 1);
    }

    #[test]
    fn clear_empties_queue() {
        let mut q = make_queue();
        q.push(item("A", "u"));
        q.push(item("B", "u"));
        q.clear();
        assert!(q.items.is_empty());
    }

    #[test]
    fn clear_empty_is_noop() {
        let mut q = make_queue();
        q.clear();
        assert!(q.items.is_empty());
    }

    #[test]
    fn serialization_preserves_persisted_state() {
        let mut q = make_queue();
        q.push(item("Saved Song", "user99"));
        let encoded = serde_json::to_string(&q).unwrap();
        let restored: VideoQueue = serde_json::from_str(&encoded).unwrap();
        assert_eq!(restored.items[0].id, q.items[0].id);
        assert_eq!(restored.items[0].title, "Saved Song");
        assert_eq!(restored.version, q.version);
    }
    #[test]
    fn stale_ack_cannot_skip_identical_consecutive_videos() {
        let mut q = make_queue();
        q.push(item("same", "u"));
        q.push(item("same", "u"));
        let id = q.items[0].id.clone();
        let version = q.version;
        assert!(q.advance_if(&id, version));
        assert!(!q.advance_if(&id, version));
        assert_eq!(q.items.len(), 1);
    }
}
