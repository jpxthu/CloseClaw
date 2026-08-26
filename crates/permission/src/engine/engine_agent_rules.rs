//! Agent-specific permission rule lazy loader.
//!
//! Resolves `{data_root}/agents/{agent_id}/permissions.json` → parses a
//! [`RuleSet`] → caches it together with O(1) lookup indices built by
//! [`build_rule_indices`]. Cache entries are keyed by `agent_id` and
//! invalidated when the file's mtime changes or the file is deleted.

use super::engine_eval::build_rule_indices;
use super::engine_types::RuleSet;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::SystemTime;
use tracing::warn;

#[allow(dead_code)]
/// Cached entry for a single agent's permission rules.
struct AgentRuleEntry {
    rules: RuleSet,
    agent_rule_index: HashMap<String, Vec<usize>>,
    user_agent_rule_index: HashMap<String, Vec<usize>>,
    mtime: SystemTime,
    /// The global rules version at the time this entry was loaded.
    global_version: String,
}

#[allow(dead_code)]
/// Lazy-loading store for per-agent permission rules.
///
/// On the first `get_or_load` call for a given `agent_id`, the store reads
/// and parses `{data_root}/agents/{agent_id}/permissions.json`, builds O(1)
/// lookup indices, and caches the result. Subsequent calls reuse the cache
/// as long as the file's mtime has not changed.
///
/// The `global_version` parameter passed to `get_or_load` tracks the global
/// rules version. When global rules are reloaded (version changes), all
/// cached entries are invalidated to ensure agent rules are re-merged with
/// the updated global rules.
///
/// The store is designed for synchronous evaluation paths and contains no
/// global state — instances are held as fields on [`PermissionEngine`](super::engine_eval::PermissionEngine).
pub(crate) struct AgentRuleStore {
    data_root: PathBuf,
    cache: HashMap<String, AgentRuleEntry>,
    /// The global rules version when the cache was last fully valid.
    /// When this differs from the current engine version, all entries
    /// must be invalidated (global rules changed, merge results stale).
    last_global_version: String,
}

#[allow(dead_code)]
impl AgentRuleStore {
    /// Create a new empty store rooted at `data_root`.
    pub fn new(data_root: PathBuf) -> Self {
        Self {
            data_root,
            cache: HashMap::new(),
            last_global_version: String::new(),
        }
    }

    /// Resolve the permissions file path for `agent_id`.
    fn agent_permissions_path(&self, agent_id: &str) -> PathBuf {
        self.data_root
            .join("agents")
            .join(agent_id)
            .join("permissions.json")
    }

    /// Get or load the cached rules and indices for `agent_id`.
    ///
    /// `global_version` is the current global rules version hash from
    /// [`RuleSet::rule_version`]. When it changes (global rules reloaded),
    /// all cached entries are invalidated to ensure agent rules are
    /// re-merged with the updated global rules.
    ///
    /// Returns `(RuleSet, agent_rule_index, user_agent_rule_index)`.
    /// If the file is missing or unparseable, an empty `RuleSet` is returned
    /// with a `warn!` log — the evaluation path continues with defaults.
    #[allow(clippy::type_complexity)]
    pub fn get_or_load(
        &mut self,
        agent_id: &str,
        global_version: &str,
    ) -> (
        RuleSet,
        HashMap<String, Vec<usize>>,
        HashMap<String, Vec<usize>>,
    ) {
        // When global rules change, all cached merge results are stale.
        if self.last_global_version != global_version {
            self.cache.clear();
            self.last_global_version = global_version.to_string();
        }

        let path = self.agent_permissions_path(agent_id);

        // Check cache freshness (mtime-based)
        let current_mtime = read_mtime(&path);
        let needs_reload = match self.cache.get(agent_id) {
            Some(entry) => {
                current_mtime.as_ref().ok() != Some(&entry.mtime)
                    || entry.global_version != global_version
            }
            None => true,
        };

        if needs_reload {
            let entry = load_agent_entry(&path, global_version);
            self.cache.insert(agent_id.to_string(), entry);
        }

        let entry = &self.cache[agent_id];
        (
            entry.rules.clone(),
            entry.agent_rule_index.clone(),
            entry.user_agent_rule_index.clone(),
        )
    }

    /// Explicitly invalidate the cached entry for `agent_id`.
    ///
    /// The next `get_or_load` call will re-read from disk.
    pub fn invalidate(&mut self, agent_id: &str) {
        self.cache.remove(agent_id);
    }

    /// Invalidate all cached entries.
    ///
    /// Called when global rules change (via `reload_rules`), so that
    /// subsequent `evaluate()` calls re-merge agent rules with the
    /// updated global rules.
    pub fn invalidate_all(&mut self) {
        self.cache.clear();
    }
}

#[allow(dead_code)]
/// Read the mtime of a file, returning `Err` if the file does not exist.
fn read_mtime(path: &Path) -> std::io::Result<SystemTime> {
    std::fs::metadata(path).and_then(|m| m.modified())
}

#[allow(dead_code)]
/// Load and parse an agent entry from disk.
///
/// On any I/O or parse error, logs a warning and returns an empty entry.
/// `global_version` is stored in the entry to track staleness when
/// global rules are reloaded.
fn load_agent_entry(path: &Path, global_version: &str) -> AgentRuleEntry {
    let mtime = read_mtime(path).unwrap_or(SystemTime::UNIX_EPOCH);

    let mut rules = match std::fs::read_to_string(path) {
        Ok(data) => match serde_json::from_str::<RuleSet>(&data) {
            Ok(rs) => rs,
            Err(e) => {
                warn!(path = %path.display(), error = %e, "failed to parse agent permissions, treating as empty");
                RuleSet::default()
            }
        },
        Err(e) => {
            warn!(path = %path.display(), error = %e, "failed to read agent permissions, treating as empty");
            RuleSet::default()
        }
    };

    rules.compute_version();
    let (agent_rule_index, user_agent_rule_index) = build_rule_indices(&rules);

    AgentRuleEntry {
        rules,
        agent_rule_index,
        user_agent_rule_index,
        mtime,
        global_version: global_version.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    fn make_ruleset_json(rules_json: &str) -> String {
        format!(
            r#"{{"rules":{},"defaults":{{"file_read":"allow","file_write":"deny","exec":"deny","network":"deny","inter_agent":"deny","config":"deny","tool_call":"deny","message":"allow"}},"user_defaults":{{"file_read":"deny","file_write":"deny","exec":"deny","network":"deny","inter_agent":"deny","config":"deny","tool_call":"deny","message":"deny"}}}}"#,
            rules_json
        )
    }

    fn setup_agent_file(dir: &Path, agent_id: &str, rules_json: &str) {
        let agent_dir = dir.join("agents").join(agent_id);
        fs::create_dir_all(&agent_dir).unwrap();
        fs::write(
            agent_dir.join("permissions.json"),
            make_ruleset_json(rules_json),
        )
        .unwrap();
    }

    #[test]
    fn load_existing_agent_rules() {
        let tmp = TempDir::new().unwrap();
        let rules_json = r#"[{"name":"allow_read","subject":{"agent":"test-agent"},"effect":"allow","actions":[{"type":"file","operation":"read","paths":["*"]}]}]"#;
        setup_agent_file(tmp.path(), "test-agent", rules_json);

        let mut store = AgentRuleStore::new(tmp.path().to_path_buf());
        let (rules, agent_idx, _user_agent_idx) = store.get_or_load("test-agent", "v1");

        assert_eq!(rules.rules.len(), 1);
        assert_eq!(rules.rules[0].name, "allow_read");
        // Index should have an entry for "test-agent"
        assert!(agent_idx.contains_key("test-agent"));
    }

    #[test]
    fn missing_file_returns_empty_ruleset() {
        let tmp = TempDir::new().unwrap();
        let mut store = AgentRuleStore::new(tmp.path().to_path_buf());
        let (rules, _, _) = store.get_or_load("nonexistent-agent", "v1");

        assert!(rules.rules.is_empty());
    }

    #[test]
    fn invalid_json_returns_empty_ruleset() {
        let tmp = TempDir::new().unwrap();
        let agent_dir = tmp.path().join("agents").join("bad-agent");
        fs::create_dir_all(&agent_dir).unwrap();
        fs::write(agent_dir.join("permissions.json"), "not valid json {{{").unwrap();

        let mut store = AgentRuleStore::new(tmp.path().to_path_buf());
        let (rules, _, _) = store.get_or_load("bad-agent", "v1");

        assert!(rules.rules.is_empty());
    }

    #[test]
    fn invalidate_forces_reload() {
        let tmp = TempDir::new().unwrap();
        let rules_v1 = r#"[{"name":"rule_v1","subject":{"agent":"a"},"effect":"allow","actions":[{"type":"file","operation":"read","paths":["*"]}]}]"#;
        setup_agent_file(tmp.path(), "a", rules_v1);

        let mut store = AgentRuleStore::new(tmp.path().to_path_buf());
        let (rules1, _, _) = store.get_or_load("a", "v1");
        assert_eq!(rules1.rules[0].name, "rule_v1");

        // Update file
        let rules_v2 = r#"[{"name":"rule_v2","subject":{"agent":"a"},"effect":"deny","actions":[{"type":"command","command":"rm"}]}]"#;
        setup_agent_file(tmp.path(), "a", rules_v2);

        // Without invalidate, cache is still valid (mtime might not change instantly)
        store.invalidate("a");
        let (rules2, _, _) = store.get_or_load("a", "v1");
        assert_eq!(rules2.rules[0].name, "rule_v2");
    }

    #[test]
    fn multiple_agents_isolated() {
        let tmp = TempDir::new().unwrap();
        setup_agent_file(
            tmp.path(),
            "agent-a",
            r#"[{"name":"rule_a","subject":{"agent":"agent-a"},"effect":"allow","actions":[{"type":"file","operation":"read","paths":["*"]}]}]"#,
        );
        setup_agent_file(
            tmp.path(),
            "agent-b",
            r#"[{"name":"rule_b","subject":{"agent":"agent-b"},"effect":"deny","actions":[{"type":"command","command":"rm"}]}]"#,
        );

        let mut store = AgentRuleStore::new(tmp.path().to_path_buf());
        let (rules_a, _, _) = store.get_or_load("agent-a", "v1");
        let (rules_b, _, _) = store.get_or_load("agent-b", "v1");

        assert_eq!(rules_a.rules[0].name, "rule_a");
        assert_eq!(rules_b.rules[0].name, "rule_b");
    }

    #[test]
    fn indices_match_rules() {
        let tmp = TempDir::new().unwrap();
        let rules_json = r#"[
            {"name":"r1","subject":{"agent":"ag1"},"effect":"allow","actions":[{"type":"file","operation":"read","paths":["*"]}]},
            {"name":"r2","subject":{"match_mode":"user_and_agent","fields":{"user_id":"u1","agent":"ag1"}},"effect":"deny","actions":[{"type":"command","command":"rm"}]}
        ]"#;
        setup_agent_file(tmp.path(), "ag1", rules_json);

        let mut store = AgentRuleStore::new(tmp.path().to_path_buf());
        let (rules, agent_idx, user_agent_idx) = store.get_or_load("ag1", "v1");

        assert_eq!(rules.rules.len(), 2);
        // ag1 should appear in agent index for both rules
        let ag_entries = agent_idx.get("ag1").unwrap();
        assert_eq!(ag_entries.len(), 2);
        // u1:ag1 should appear in user_agent index
        assert!(user_agent_idx.contains_key("u1:ag1"));
    }

    #[test]
    fn global_version_change_invalidates_all_entries() {
        let tmp = TempDir::new().unwrap();
        let rules_v1 = r#"[{"name":"rule_v1","subject":{"agent":"a"},"effect":"allow","actions":[{"type":"file","operation":"read","paths":["*"]}]}]"#;
        setup_agent_file(tmp.path(), "a", rules_v1);

        let mut store = AgentRuleStore::new(tmp.path().to_path_buf());
        let (rules1, _, _) = store.get_or_load("a", "global-v1");
        assert_eq!(rules1.rules[0].name, "rule_v1");

        // Update file content
        let rules_v2 = r#"[{"name":"rule_v2","subject":{"agent":"a"},"effect":"deny","actions":[{"type":"command","command":"rm"}]}]"#;
        setup_agent_file(tmp.path(), "a", rules_v2);

        // Different global version → all entries invalidated, re-reads from disk
        let (rules2, _, _) = store.get_or_load("a", "global-v2");
        assert_eq!(rules2.rules[0].name, "rule_v2");
    }

    #[test]
    fn global_version_change_clears_all_entries() {
        let tmp = TempDir::new().unwrap();
        // Set up two agents
        setup_agent_file(
            tmp.path(),
            "agent-x",
            r#"[{"name":"rule_x","subject":{"agent":"agent-x"},"effect":"allow","actions":[{"type":"file","operation":"read","paths":["*"]}]}]"#,
        );
        setup_agent_file(
            tmp.path(),
            "agent-y",
            r#"[{"name":"rule_y","subject":{"agent":"agent-y"},"effect":"deny","actions":[{"type":"command","command":"rm"}]}]"#,
        );

        let mut store = AgentRuleStore::new(tmp.path().to_path_buf());
        // Load both agents with v1
        let (rx, _, _) = store.get_or_load("agent-x", "v1");
        let (ry, _, _) = store.get_or_load("agent-y", "v1");
        assert_eq!(rx.rules[0].name, "rule_x");
        assert_eq!(ry.rules[0].name, "rule_y");

        // Change agent-x rules
        setup_agent_file(
            tmp.path(),
            "agent-x",
            r#"[{"name":"rule_x2","subject":{"agent":"agent-x"},"effect":"deny","actions":[{"type":"command","command":"rm"}]}]"#,
        );

        // global-v2 invalidates ALL entries, even agent-y (which hasn't changed)
        let (rx2, _, _) = store.get_or_load("agent-x", "v2");
        let (ry2, _, _) = store.get_or_load("agent-y", "v2");
        assert_eq!(rx2.rules[0].name, "rule_x2");
        assert_eq!(ry2.rules[0].name, "rule_y");
    }
}
