use std::collections::HashMap;
use std::time::{Duration, Instant};

pub struct CooldownManager {
    per_user: HashMap<(String, String), Instant>,
    global:   HashMap<String, Instant>,
}

impl CooldownManager {
    pub fn new() -> Self {
        Self { per_user: HashMap::new(), global: HashMap::new() }
    }

    /// true → puede usar; false → en cooldown.
    pub fn check_user(&self, user: &str, cmd: &str, secs: u64) -> bool {
        match self.per_user.get(&(user.to_lowercase(), cmd.to_string())) {
            Some(t) => t.elapsed() >= Duration::from_secs(secs),
            None    => true,
        }
    }

    pub fn use_user(&mut self, user: &str, cmd: &str) {
        self.per_user.insert((user.to_lowercase(), cmd.to_string()), Instant::now());
    }

    pub fn check_global(&self, cmd: &str, secs: u64) -> bool {
        match self.global.get(cmd) {
            Some(t) => t.elapsed() >= Duration::from_secs(secs),
            None    => true,
        }
    }

    pub fn use_global(&mut self, cmd: &str) {
        self.global.insert(cmd.to_string(), Instant::now());
    }

    #[allow(dead_code)]
    pub fn gc(&mut self, max_secs: u64) {
        let limit = Duration::from_secs(max_secs);
        self.per_user.retain(|_, t| t.elapsed() < limit);
        self.global.retain(|_, t| t.elapsed() < limit);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn user_not_on_cooldown_initially() {
        let cd = CooldownManager::new();
        assert!(cd.check_user("user1", "!dado", 15));
    }

    #[test]
    fn user_on_cooldown_immediately_after_use() {
        let mut cd = CooldownManager::new();
        cd.use_user("user1", "!dado");
        assert!(!cd.check_user("user1", "!dado", 15));
    }

    #[test]
    fn zero_secs_cooldown_always_passes() {
        let mut cd = CooldownManager::new();
        cd.use_user("user1", "!dado");
        assert!(cd.check_user("user1", "!dado", 0));
    }

    #[test]
    fn different_users_are_independent() {
        let mut cd = CooldownManager::new();
        cd.use_user("user1", "!dado");
        assert!(!cd.check_user("user1", "!dado", 15));
        assert!(cd.check_user("user2", "!dado", 15));
    }

    #[test]
    fn different_commands_are_independent() {
        let mut cd = CooldownManager::new();
        cd.use_user("user1", "!dado");
        assert!(!cd.check_user("user1", "!dado", 15));
        assert!(cd.check_user("user1", "!8ball", 15));
    }

    #[test]
    fn username_is_case_insensitive() {
        let mut cd = CooldownManager::new();
        cd.use_user("SeniorDai", "!s");
        assert!(!cd.check_user("seniordai", "!s", 15));
        assert!(!cd.check_user("SENIORDAI", "!s", 15));
    }

    #[test]
    fn global_cooldown_starts_free() {
        let cd = CooldownManager::new();
        assert!(cd.check_global("!uptime", 30));
    }

    #[test]
    fn global_cooldown_blocks_after_use() {
        let mut cd = CooldownManager::new();
        cd.use_global("!uptime");
        assert!(!cd.check_global("!uptime", 30));
    }

    #[test]
    fn different_global_commands_are_independent() {
        let mut cd = CooldownManager::new();
        cd.use_global("!uptime");
        assert!(!cd.check_global("!uptime", 30));
        assert!(cd.check_global("!cola", 30));
    }

    #[test]
    fn gc_with_large_timeout_keeps_fresh_entries() {
        let mut cd = CooldownManager::new();
        cd.use_user("user1", "!dado");
        cd.use_global("!uptime");
        cd.gc(3600);
        assert_eq!(cd.per_user.len(), 1);
        assert_eq!(cd.global.len(), 1);
    }

    #[test]
    fn gc_with_zero_timeout_removes_all() {
        let mut cd = CooldownManager::new();
        cd.use_user("user1", "!dado");
        cd.use_user("user2", "!s");
        cd.use_global("!uptime");
        cd.gc(0);
        assert_eq!(cd.per_user.len(), 0);
        assert_eq!(cd.global.len(), 0);
    }
}
