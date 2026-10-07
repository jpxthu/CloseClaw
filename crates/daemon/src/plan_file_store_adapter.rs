//! Production [`PlanFileStore`] implementation for the daemon
//! composition root.
//!
//! [`SessionPlanFileStore`] is a stateless adapter over the
//! `closeclaw_session::plan_file` functions: slash-side mirror types
//! (identifier format, resolve errors, summaries) are mapped to and
//! from the session-side originals at the boundary. Error `Display`
//! strings are preserved verbatim so reply texts are unchanged.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use closeclaw_session::plan_file as session_plan_file;
use closeclaw_slash::plan_file_store::{
    PlanFileStore, PlanNameFormat, PlanResolveError, PlanSummary,
};

/// Map the slash-side identifier format to the session-side enum.
fn format_to_session(format: PlanNameFormat) -> session_plan_file::PlanIdentifierFormat {
    match format {
        PlanNameFormat::Timestamp => session_plan_file::PlanIdentifierFormat::Timestamp,
        PlanNameFormat::RandomWords => session_plan_file::PlanIdentifierFormat::RandomWords,
    }
}

/// Map the session-side resolve error to the slash-side mirror,
/// preserving the `name` / `candidates` payloads (and therefore the
/// `Display` output) verbatim.
fn error_to_slash(e: session_plan_file::PlanResolveError) -> PlanResolveError {
    match e {
        session_plan_file::PlanResolveError::NotFound { name } => {
            PlanResolveError::NotFound { name }
        }
        session_plan_file::PlanResolveError::Ambiguous { name, candidates } => {
            PlanResolveError::Ambiguous { name, candidates }
        }
    }
}

/// Map a session-side summary to the slash-side mirror.
fn summary_to_slash(s: session_plan_file::PlanSummary) -> PlanSummary {
    PlanSummary {
        stem: s.stem,
        title: s.title,
        completed: s.completed,
        failed: s.failed,
        skipped: s.skipped,
        total: s.total,
    }
}

/// Stateless production adapter wrapping the session plan-file functions.
pub struct SessionPlanFileStore;

/// Build the production plan-file store as `Arc<dyn PlanFileStore>`.
pub fn session_plan_file_store() -> Arc<dyn PlanFileStore> {
    Arc::new(SessionPlanFileStore)
}

impl PlanFileStore for SessionPlanFileStore {
    fn create_plan_file_with_format(
        &self,
        workdir: &Path,
        title: &str,
        format: PlanNameFormat,
    ) -> Result<PathBuf, std::io::Error> {
        session_plan_file::create_plan_file_with_format(workdir, title, format_to_session(format))
    }

    fn resolve_plan_by_name(
        &self,
        workdir: &Path,
        name: &str,
    ) -> Result<PathBuf, PlanResolveError> {
        session_plan_file::resolve_plan_by_name(workdir, name).map_err(error_to_slash)
    }

    fn touch_access_timestamp(&self, plan_path: &Path) -> Result<(), std::io::Error> {
        session_plan_file::touch_access_timestamp(plan_path)
    }

    fn list_plan_summaries(&self, workdir: &Path) -> std::io::Result<Vec<PlanSummary>> {
        session_plan_file::list_plan_summaries(workdir)
            .map(|v| v.into_iter().map(summary_to_slash).collect())
    }

    fn read_plan_content(&self, path: &Path) -> std::io::Result<String> {
        session_plan_file::read_plan_content(path)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── Enum mapping (正常路径) ────────────────────────────────────────

    #[test]
    fn format_mapping_covers_all_variants() {
        assert_eq!(
            format_to_session(PlanNameFormat::Timestamp),
            session_plan_file::PlanIdentifierFormat::Timestamp
        );
        assert_eq!(
            format_to_session(PlanNameFormat::RandomWords),
            session_plan_file::PlanIdentifierFormat::RandomWords
        );
        // Default agreement: both sides default to Timestamp.
        assert_eq!(
            format_to_session(PlanNameFormat::default()),
            session_plan_file::PlanIdentifierFormat::default()
        );
    }

    // ── Error mapping（错误/边界：NotFound、Ambiguous 多候选） ─────────

    #[test]
    fn not_found_error_mapping_preserves_payload_and_display() {
        let session_err = session_plan_file::PlanResolveError::NotFound {
            name: "missing-plan".to_string(),
        };
        // Capture the session-side Display before the error is moved.
        let session_display = session_err.to_string();
        match error_to_slash(session_err) {
            PlanResolveError::NotFound { name } => assert_eq!(name, "missing-plan"),
            other => panic!("expected NotFound, got {other:?}"),
        }
        // Display must be byte-identical so reply text is unchanged.
        assert_eq!(session_display, "plan not found: missing-plan");
        let slash_display = PlanResolveError::NotFound {
            name: "missing-plan".to_string(),
        }
        .to_string();
        assert_eq!(slash_display, session_display);
    }

    #[test]
    fn ambiguous_error_mapping_preserves_payload_and_display() {
        let session_err = session_plan_file::PlanResolveError::Ambiguous {
            name: "auth".to_string(),
            candidates: vec!["auth-login".to_string(), "auth-logout".to_string()],
        };
        let session_display = session_err.to_string();
        match error_to_slash(session_err) {
            PlanResolveError::Ambiguous { name, candidates } => {
                assert_eq!(name, "auth");
                assert_eq!(candidates, vec!["auth-login", "auth-logout"]);
            }
            other => panic!("expected Ambiguous, got {other:?}"),
        }
        let slash_display = PlanResolveError::Ambiguous {
            name: "auth".to_string(),
            candidates: vec!["auth-login".to_string(), "auth-logout".to_string()],
        }
        .to_string();
        assert_eq!(slash_display, session_display);
    }

    // ── Adapter behavior against the real session functions ──────────

    #[test]
    fn create_and_resolve_roundtrip_for_both_formats() {
        for format in [PlanNameFormat::Timestamp, PlanNameFormat::RandomWords] {
            let tmp = tempfile::TempDir::new().unwrap();
            let store = SessionPlanFileStore;
            let path = store
                .create_plan_file_with_format(tmp.path(), "implement feature", format)
                .expect("create plan file");
            assert!(path.exists(), "created plan file should exist");

            // Resolve returns the session-side relative path form
            // (`plans/{stem}.md`) — handlers join it with the workdir.
            let stem = path.file_stem().unwrap().to_string_lossy();
            let resolved = store
                .resolve_plan_by_name(tmp.path(), &stem)
                .expect("exact resolve");
            assert_eq!(
                resolved,
                PathBuf::from(format!("plans/{stem}.md")),
                "resolve should return the relative plans/ path"
            );
            assert!(tmp.path().join(&resolved).exists());

            // Prefix + fuzzy resolution still work through the adapter.
            let prefix = &stem[..stem.len() / 2];
            assert!(store.resolve_plan_by_name(tmp.path(), prefix).is_ok());
        }
    }

    #[test]
    fn resolve_not_found_and_ambiguous_surfaces_slash_error() {
        let tmp = tempfile::TempDir::new().unwrap();
        let store = SessionPlanFileStore;

        // Missing plans directory → session reports NotFound with an
        // empty name (quirk preserved verbatim through the adapter).
        match store.resolve_plan_by_name(tmp.path(), "nonexistent") {
            Err(PlanResolveError::NotFound { name }) => {
                assert_eq!(name, "", "missing plans dir yields empty-name NotFound");
            }
            other => panic!("expected NotFound, got {other:?}"),
        }

        // Empty plans directory → NotFound carrying the queried name.
        std::fs::create_dir_all(tmp.path().join("plans")).unwrap();
        match store.resolve_plan_by_name(tmp.path(), "nonexistent") {
            Err(PlanResolveError::NotFound { name }) => assert_eq!(name, "nonexistent"),
            other => panic!("expected NotFound, got {other:?}"),
        }

        // Two prefix-matching plans → Ambiguous with both candidates.
        let plans = tmp.path().join("plans");
        std::fs::write(plans.join("auth-login.md"), "# Login\n").unwrap();
        std::fs::write(plans.join("auth-logout.md"), "# Logout\n").unwrap();
        match store.resolve_plan_by_name(tmp.path(), "auth") {
            Err(PlanResolveError::Ambiguous { name, candidates }) => {
                assert_eq!(name, "auth");
                assert_eq!(candidates.len(), 2);
                assert!(candidates.contains(&"auth-login".to_string()));
                assert!(candidates.contains(&"auth-logout".to_string()));
            }
            other => panic!("expected Ambiguous, got {other:?}"),
        }
    }

    #[test]
    fn touch_list_read_roundtrip_preserves_summary_fields() {
        let tmp = tempfile::TempDir::new().unwrap();
        let plans = tmp.path().join("plans");
        std::fs::create_dir_all(&plans).unwrap();
        std::fs::write(
            plans.join("alpha.md"),
            "# Alpha Feature\n\n## Tasks\n\n- [x] Step 1\n- [!] Step 2\n- [~] Step 3\n- [ ] Step 4\n",
        )
        .unwrap();

        let store = SessionPlanFileStore;
        // touch_access_timestamp succeeds on an existing plan file.
        store
            .touch_access_timestamp(&plans.join("alpha.md"))
            .expect("touch access timestamp");

        let summaries = store.list_plan_summaries(tmp.path()).expect("summaries");
        assert_eq!(summaries.len(), 1);
        let s = &summaries[0];
        assert_eq!(s.stem, "alpha");
        assert_eq!(s.title, "Alpha Feature");
        assert_eq!(s.completed, 1);
        assert_eq!(s.failed, 1);
        assert_eq!(s.skipped, 1);
        assert_eq!(s.total, 4);

        let content = store
            .read_plan_content(&plans.join("alpha.md"))
            .expect("read plan content");
        assert!(content.contains("# Alpha Feature"));

        // Missing plans directory → empty listing (not an error).
        let empty = tempfile::TempDir::new().unwrap();
        assert!(store.list_plan_summaries(empty.path()).unwrap().is_empty());
    }

    #[test]
    fn factory_returns_trait_object() {
        let store = session_plan_file_store();
        let tmp = tempfile::TempDir::new().unwrap();
        assert!(store.list_plan_summaries(tmp.path()).unwrap().is_empty());
    }
}
