//! Agent-specific permission rule lazy loader.
//!
//! Resolves `{data_root}/agents/{agent_id}/permissions.json` → parses a
//! [`RuleSet`] → merges with global rules → caches the merged result
//! together with O(1) lookup indices built by [`build_rule_indices`].
//! Cache entries are keyed by `agent_id` and invalidated when the file's
//! mtime changes or the file is deleted.

use super::engine_eval::build_rule_indices;
use super::engine_types::RuleSet;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::SystemTime;
use tracing::warn;

/// Pre-built merge of global + agent rules with O(1) lookup indices.
///
/// Cached per-agent so that repeated `evaluate()` calls reuse the merged
/// result without re-cloning the global `RuleSet` or rebuilding indices.
#[derive(Clone, Default)]
pub(crate) struct MergedAgentRules {
    /// Combined global + agent rules.
    pub rules: RuleSet,
    /// O(1) agent lookup index for the merged set.
    pub agent_rule_index: HashMap<String, Vec<usize>>,
    /// O(1) user+agent lookup index for the merged set.
    pub user_agent_rule_index: HashMap<String, Vec<usize>>,
}

/// Cached entry for a single agent's permission rules.
struct AgentRuleEntry {
    /// Pre-built merged result (global + agent rules).
    merged: MergedAgentRules,
    /// Number of agent-specific rules (for logging).
    agent_rule_count: usize,
    mtime: SystemTime,
    /// The global rules version at the time this entry was loaded.
    global_version: String,
}

/// Lazy-loading store for per-agent permission rules.
///
/// On the first `get_or_load` call for a given `agent_id`, the store reads
/// and parses `{data_root}/agents/{agent_id}/permissions.json`, merges with
/// the global rules, builds O(1) lookup indices, and caches the result.
/// Subsequent calls reuse the cache as long as the file's mtime has not
/// changed and global rules have not been reloaded.
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

    /// Get or load the cached merged rules for `agent_id`.
    ///
    /// `global_version` is the current global rules version hash from
    /// [`RuleSet::rule_version`]. When it changes (global rules reloaded),
    /// all cached entries are invalidated to ensure agent rules are
    /// re-merged with the updated global rules.
    ///
    /// Returns `(MergedAgentRules, agent_rule_count)` where the merged
    /// rules include both global and agent-specific rules with pre-built
    /// O(1) indices. If the file is missing or unparseable, the merged
    /// rules contain only the global rules (agent rules empty).
    ///
    /// The caller should `put()` the result back after evaluation to
    /// avoid losing the cache entry (swap pattern). See
    /// [`PermissionEngine::evaluate`](super::engine_eval::PermissionEngine::evaluate).
    pub fn get_or_load(
        &mut self,
        agent_id: &str,
        global_rules: &RuleSet,
        global_version: &str,
    ) -> (MergedAgentRules, usize) {
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
            let entry = load_agent_entry(&path, global_rules, global_version);
            self.cache.insert(agent_id.to_string(), entry);
        }

        let entry = &self.cache[agent_id];
        let count = entry.agent_rule_count;
        (entry.merged.clone(), count)
    }

    /// Put a merged entry back into the cache after evaluation.
    ///
    /// Only updates if the entry still exists (hasn't been invalidated).
    pub fn put(&mut self, agent_id: &str, merged: MergedAgentRules, agent_rule_count: usize) {
        if let Some(entry) = self.cache.get_mut(agent_id) {
            entry.merged = merged;
            entry.agent_rule_count = agent_rule_count;
        }
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

/// Read the mtime of a file, returning `Err` if the file does not exist.
fn read_mtime(path: &Path) -> std::io::Result<SystemTime> {
    std::fs::metadata(path).and_then(|m| m.modified())
}

/// Load and parse an agent entry from disk, merge with global rules, and
/// build O(1) indices.
///
/// On any I/O or parse error, logs a warning and returns an empty merge
/// (global rules only, no agent rules). `global_version` is stored in the
/// entry to track staleness when global rules are reloaded.
fn load_agent_entry(path: &Path, global_rules: &RuleSet, global_version: &str) -> AgentRuleEntry {
    let mtime = read_mtime(path).unwrap_or(SystemTime::UNIX_EPOCH);

    let agent_rules = match std::fs::read_to_string(path) {
        Ok(data) => match serde_json::from_str::<RuleSet>(&data) {
            Ok(mut rs) => {
                rs.compute_version();
                rs
            }
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

    let agent_rule_count = agent_rules.rules.len();

    // Merge: append agent rules to a clone of the global rules
    let mut merged_rules = global_rules.clone();
    merged_rules.rules.extend(agent_rules.rules);

    // Build combined indices from the merged ruleset
    let (agent_rule_index, user_agent_rule_index) = build_rule_indices(&merged_rules);

    AgentRuleEntry {
        merged: MergedAgentRules {
            rules: merged_rules,
            agent_rule_index,
            user_agent_rule_index,
        },
        agent_rule_count,
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

    fn empty_global_rules() -> RuleSet {
        RuleSet::default()
    }

    #[test]
    fn load_existing_agent_rules() {
        let tmp = TempDir::new().unwrap();
        let rules_json = r#"[{"name":"allow_read","subject":{"agent":"test-agent"},"effect":"allow","actions":[{"type":"file","operation":"read","paths":["*"]}]}]"#;
        setup_agent_file(tmp.path(), "test-agent", rules_json);

        let global = empty_global_rules();
        let mut store = AgentRuleStore::new(tmp.path().to_path_buf());
        let (merged, count) = store.get_or_load("test-agent", &global, "v1");

        assert_eq!(merged.rules.rules.len(), 1);
        assert_eq!(merged.rules.rules[0].name, "allow_read");
        assert_eq!(count, 1);
        // Index should have an entry for "test-agent"
        assert!(merged.agent_rule_index.contains_key("test-agent"));
    }

    #[test]
    fn missing_file_returns_global_only() {
        let tmp = TempDir::new().unwrap();
        let global = empty_global_rules();
        let mut store = AgentRuleStore::new(tmp.path().to_path_buf());
        let (merged, count) = store.get_or_load("nonexistent-agent", &global, "v1");

        assert!(merged.rules.rules.is_empty());
        assert_eq!(count, 0);
    }

    #[test]
    fn invalid_json_returns_global_only() {
        let tmp = TempDir::new().unwrap();
        let agent_dir = tmp.path().join("agents").join("bad-agent");
        fs::create_dir_all(&agent_dir).unwrap();
        fs::write(agent_dir.join("permissions.json"), "not valid json {{{").unwrap();

        let global = empty_global_rules();
        let mut store = AgentRuleStore::new(tmp.path().to_path_buf());
        let (merged, count) = store.get_or_load("bad-agent", &global, "v1");

        assert!(merged.rules.rules.is_empty());
        assert_eq!(count, 0);
    }

    #[test]
    fn invalidate_forces_reload() {
        let tmp = TempDir::new().unwrap();
        let rules_v1 = r#"[{"name":"rule_v1","subject":{"agent":"a"},"effect":"allow","actions":[{"type":"file","operation":"read","paths":["*"]}]}]"#;
        setup_agent_file(tmp.path(), "a", rules_v1);

        let global = empty_global_rules();
        let mut store = AgentRuleStore::new(tmp.path().to_path_buf());
        let (merged1, _) = store.get_or_load("a", &global, "v1");
        assert_eq!(merged1.rules.rules[0].name, "rule_v1");

        // Update file
        let rules_v2 = r#"[{"name":"rule_v2","subject":{"agent":"a"},"effect":"deny","actions":[{"type":"command","command":"rm"}]}]"#;
        setup_agent_file(tmp.path(), "a", rules_v2);

        // Without invalidate, cache is still valid (mtime might not change instantly)
        store.invalidate("a");
        let (merged2, _) = store.get_or_load("a", &global, "v1");
        assert_eq!(merged2.rules.rules[0].name, "rule_v2");
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

        let global = empty_global_rules();
        let mut store = AgentRuleStore::new(tmp.path().to_path_buf());
        let (merged_a, _) = store.get_or_load("agent-a", &global, "v1");
        let (merged_b, _) = store.get_or_load("agent-b", &global, "v1");

        assert_eq!(merged_a.rules.rules[0].name, "rule_a");
        assert_eq!(merged_b.rules.rules[0].name, "rule_b");
    }

    #[test]
    fn indices_match_rules() {
        let tmp = TempDir::new().unwrap();
        let rules_json = r#"[
            {"name":"r1","subject":{"agent":"ag1"},"effect":"allow","actions":[{"type":"file","operation":"read","paths":["*"]}]},
            {"name":"r2","subject":{"match_mode":"user_and_agent","fields":{"user_id":"u1","agent":"ag1"}},"effect":"deny","actions":[{"type":"command","command":"rm"}]}
        ]"#;
        setup_agent_file(tmp.path(), "ag1", rules_json);

        let global = empty_global_rules();
        let mut store = AgentRuleStore::new(tmp.path().to_path_buf());
        let (merged, _) = store.get_or_load("ag1", &global, "v1");

        assert_eq!(merged.rules.rules.len(), 2);
        // ag1 should appear in agent index for both rules
        let ag_entries = merged.agent_rule_index.get("ag1").unwrap();
        assert_eq!(ag_entries.len(), 2);
        // u1:ag1 should appear in user_agent index
        assert!(merged.user_agent_rule_index.contains_key("u1:ag1"));
    }

    #[test]
    fn global_version_change_invalidates_all_entries() {
        let tmp = TempDir::new().unwrap();
        let rules_v1 = r#"[{"name":"rule_v1","subject":{"agent":"a"},"effect":"allow","actions":[{"type":"file","operation":"read","paths":["*"]}]}]"#;
        setup_agent_file(tmp.path(), "a", rules_v1);

        let global = empty_global_rules();
        let mut store = AgentRuleStore::new(tmp.path().to_path_buf());
        let (merged1, _) = store.get_or_load("a", &global, "global-v1");
        assert_eq!(merged1.rules.rules[0].name, "rule_v1");

        // Update file content
        let rules_v2 = r#"[{"name":"rule_v2","subject":{"agent":"a"},"effect":"deny","actions":[{"type":"command","command":"rm"}]}]"#;
        setup_agent_file(tmp.path(), "a", rules_v2);

        // Different global version → all entries invalidated, re-reads from disk
        let (merged2, _) = store.get_or_load("a", &global, "global-v2");
        assert_eq!(merged2.rules.rules[0].name, "rule_v2");
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

        let global = empty_global_rules();
        let mut store = AgentRuleStore::new(tmp.path().to_path_buf());
        // Load both agents with v1
        let (mx, _) = store.get_or_load("agent-x", &global, "v1");
        let (my, _) = store.get_or_load("agent-y", &global, "v1");
        assert_eq!(mx.rules.rules[0].name, "rule_x");
        assert_eq!(my.rules.rules[0].name, "rule_y");

        // Change agent-x rules
        setup_agent_file(
            tmp.path(),
            "agent-x",
            r#"[{"name":"rule_x2","subject":{"agent":"agent-x"},"effect":"deny","actions":[{"type":"command","command":"rm"}]}]"#,
        );

        // global-v2 invalidates ALL entries, even agent-y (which hasn't changed)
        let (mx2, _) = store.get_or_load("agent-x", &global, "v2");
        let (my2, _) = store.get_or_load("agent-y", &global, "v2");
        assert_eq!(mx2.rules.rules[0].name, "rule_x2");
        assert_eq!(my2.rules.rules[0].name, "rule_y");
    }

    #[test]
    fn merged_includes_global_rules() {
        let tmp = TempDir::new().unwrap();
        setup_agent_file(
            tmp.path(),
            "agent-g",
            r#"[{"name":"agent_rule","subject":{"agent":"agent-g"},"effect":"allow","actions":[{"type":"file","operation":"read","paths":["*"]}]}]"#,
        );

        // Global has its own rule
        let mut global = RuleSet::default();
        global.rules.push(crate::engine::engine_types::Rule {
            name: "global_rule".to_string(),
            subject: crate::engine::engine_types::Subject::AgentOnly {
                agent: "agent-g".to_string(),
                match_type: crate::engine::engine_types::MatchType::Exact,
            },
            effect: crate::engine::engine_types::Effect::Deny,
            actions: vec![crate::engine::engine_types::Action::File {
                operation: "write".to_string(),
                paths: vec!["/**".to_string()],
            }],
            template: None,
            priority: 0,
        });

        let mut store = AgentRuleStore::new(tmp.path().to_path_buf());
        let (merged, count) = store.get_or_load("agent-g", &global, "v1");

        // Should have both global + agent rules
        assert_eq!(merged.rules.rules.len(), 2);
        assert_eq!(count, 1); // only 1 agent rule
                              // Global rule preserved
        assert!(merged.rules.rules.iter().any(|r| r.name == "global_rule"));
        // Agent rule present
        assert!(merged.rules.rules.iter().any(|r| r.name == "agent_rule"));
    }

    #[test]
    fn put_restores_cache_entry() {
        let tmp = TempDir::new().unwrap();
        setup_agent_file(
            tmp.path(),
            "put-agent",
            r#"[{"name":"put_rule","subject":{"agent":"put-agent"},"effect":"allow","actions":[{"type":"file","operation":"read","paths":["*"]}]}]"#,
        );

        let global = empty_global_rules();
        let mut store = AgentRuleStore::new(tmp.path().to_path_buf());
        let (merged, count) = store.get_or_load("put-agent", &global, "v1");
        assert_eq!(count, 1);

        // Put it back
        store.put("put-agent", merged, count);

        // Next get should still have the cached entry (no reload)
        let (merged2, count2) = store.get_or_load("put-agent", &global, "v1");
        assert_eq!(count2, 1);
        assert_eq!(merged2.rules.rules[0].name, "put_rule");
    }
}
