use serde::{Deserialize, Serialize};
use std::fs;
use tracing::{error, info};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VideoItem {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub video_id: Option<String>,   // → "videoId" en JSON
    #[serde(skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    pub title: String,
    pub user: String,
}

#[derive(Debug)]
pub struct VideoQueue {
    pub items: Vec<VideoItem>,
    queue_file: String,
}

impl VideoQueue {
    pub fn load(queue_file: &str) -> Self {
        let items: Vec<VideoItem> = fs::read_to_string(queue_file)
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default();

        if !items.is_empty() {
            info!("Cola cargada: {} videos", items.len());
        }
        Self { items, queue_file: queue_file.to_string() }
    }

    pub fn push(&mut self, item: VideoItem) {
        self.items.push(item);
        self.save();
    }

    pub fn advance(&mut self) {
        if !self.items.is_empty() {
            self.items.remove(0);
            self.save();
        }
    }

    pub fn remove(&mut self, index: usize) {
        if index < self.items.len() {
            self.items.remove(index);
            self.save();
        }
    }

    pub fn clear(&mut self) {
        self.items.clear();
        self.save();
    }

    pub fn save(&self) {
        if let Some(parent) = std::path::Path::new(&self.queue_file).parent() {
            let _ = fs::create_dir_all(parent);
        }
        match serde_json::to_string_pretty(&self.items) {
            Ok(json) => { if let Err(e) = fs::write(&self.queue_file, json) { error!("Error guardando cola: {e}"); } }
            Err(e) => error!("Error serializando cola: {e}"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU32, Ordering};

    static COUNTER: AtomicU32 = AtomicU32::new(0);

    fn make_queue() -> VideoQueue {
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!("test_queue_{n}.json"));
        let path = path.to_string_lossy().into_owned();
        fs::write(&path, "[]").unwrap();
        VideoQueue::load(&path)
    }

    fn item(title: &str, user: &str) -> VideoItem {
        VideoItem {
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
    fn persists_to_file_and_reloads() {
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!("test_queue_persist_{n}.json"));
        let path = path.to_string_lossy().into_owned();
        fs::write(&path, "[]").unwrap();
        {
            let mut q = VideoQueue::load(&path);
            q.push(item("Saved Song", "user99"));
        }
        let q2 = VideoQueue::load(&path);
        assert_eq!(q2.items.len(), 1);
        assert_eq!(q2.items[0].title, "Saved Song");
    }
}
