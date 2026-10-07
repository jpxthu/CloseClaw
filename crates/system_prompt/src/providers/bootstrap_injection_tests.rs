//! Step 1.6 injection-contract tests for BootstrapFragmentProvider.
//!
//! Fake lister/loader fn pointers verify the injected-dependency contract:
//! lister ordering, loader-failure tolerance, cache_key loader isolation,
//! and mode propagation (Sub → Minimal downgrade).

use super::*;
use std::fs;
use std::sync::atomic::{AtomicUsize, Ordering};

// ============================================================
// Step 1.6: injection contract tests (fake lister + loader)
// ============================================================

fn injection_lister(mode: BootstrapMode) -> Vec<&'static str> {
    match mode {
        BootstrapMode::Full => vec!["ALPHA.md", "BETA.md"],
        BootstrapMode::Minimal => vec!["ALPHA.md"],
    }
}

/// Loader returns ALPHA.md, BETA.md and an extra GAMMA.md that the
/// lister never reports — GAMMA.md must be skipped by generate().
fn injection_loader(_dir: &Path, _mode: BootstrapMode) -> Option<HashMap<String, String>> {
    let mut files = HashMap::new();
    files.insert("ALPHA.md".to_string(), "alpha body".to_string());
    files.insert("BETA.md".to_string(), "beta body".to_string());
    files.insert("GAMMA.md".to_string(), "gamma body".to_string());
    Some(files)
}

/// generate() must emit files in lister order (not loader-map order)
/// and skip loader-produced entries missing from the lister list.
#[tokio::test]
async fn test_generate_injected_lister_order_skips_unlisted() {
    let provider = BootstrapFragmentProvider::new(injection_lister, injection_loader);
    let ctx = FragmentContext {
        bootstrap_dir: "/unused".to_string(),
        bootstrap_mode: BootstrapMode::Full,
        ..FragmentContext::test_default()
    };
    let fragment = provider.generate(&ctx).await.unwrap();
    assert_eq!(
        fragment.content,
        "## ALPHA.md\nalpha body\n\n## BETA.md\nbeta body"
    );
    assert!(
        !fragment.content.contains("GAMMA"),
        "entries absent from the lister list must be skipped"
    );
}

fn failing_loader(_dir: &Path, _mode: BootstrapMode) -> Option<HashMap<String, String>> {
    None
}

/// Loader returning None (load failure) → generate() yields no
/// fragment instead of panicking.
#[tokio::test]
async fn test_generate_loader_failure_returns_no_fragment() {
    let provider = BootstrapFragmentProvider::new(injection_lister, failing_loader);
    let ctx = FragmentContext {
        bootstrap_dir: "/unused".to_string(),
        bootstrap_mode: BootstrapMode::Full,
        ..FragmentContext::test_default()
    };
    assert!(provider.generate(&ctx).await.is_none());
}

static CACHE_PROBE_LIST_CALLS: AtomicUsize = AtomicUsize::new(0);
static CACHE_PROBE_LOAD_CALLS: AtomicUsize = AtomicUsize::new(0);

fn counting_lister(_mode: BootstrapMode) -> Vec<&'static str> {
    CACHE_PROBE_LIST_CALLS.fetch_add(1, Ordering::SeqCst);
    vec!["AGENTS.md"]
}

fn counting_loader(_dir: &Path, _mode: BootstrapMode) -> Option<HashMap<String, String>> {
    CACHE_PROBE_LOAD_CALLS.fetch_add(1, Ordering::SeqCst);
    Some(HashMap::new())
}

/// cache_key() must take the file list from the lister and never
/// invoke the (fs-reading) loader — asserted via call counters.
#[tokio::test]
async fn test_cache_key_does_not_invoke_loader() {
    let tmp = tempfile::tempdir().unwrap();
    fs::write(tmp.path().join("AGENTS.md"), "content").unwrap();

    let provider = BootstrapFragmentProvider::new(counting_lister, counting_loader);
    let ctx = FragmentContext {
        bootstrap_dir: tmp.path().to_string_lossy().to_string(),
        bootstrap_mode: BootstrapMode::Minimal,
        ..FragmentContext::test_default()
    };

    let key = provider.cache_key(&ctx).await;
    assert!(key.is_some(), "cache key must exist for the listed file");
    assert_eq!(
        CACHE_PROBE_LOAD_CALLS.load(Ordering::SeqCst),
        0,
        "cache_key must not invoke the injected loader"
    );
    assert_eq!(
        CACHE_PROBE_LIST_CALLS.load(Ordering::SeqCst),
        1,
        "cache_key must take the file list from the injected lister"
    );
}

/// Both injected fns echo the mode they received into their output, so
/// the fragment content proves which mode each was called with.
fn mode_echo_lister(mode: BootstrapMode) -> Vec<&'static str> {
    match mode {
        BootstrapMode::Full => vec!["BOOTSTRAP.md"],
        BootstrapMode::Minimal => vec!["AGENTS.md"],
    }
}

fn mode_echo_loader(_dir: &Path, mode: BootstrapMode) -> Option<HashMap<String, String>> {
    let name = match mode {
        BootstrapMode::Full => "BOOTSTRAP.md",
        BootstrapMode::Minimal => "AGENTS.md",
    };
    let mut files = HashMap::new();
    files.insert(name.to_string(), format!("mode={mode:?}"));
    Some(files)
}

/// Sub agent configured with Full mode → role guard downgrades to
/// Minimal: lister and loader must both be called with Minimal
/// (dependency inversion must not change mode semantics).
#[tokio::test]
async fn test_sub_role_passes_minimal_mode_to_injected_fns() {
    let provider = BootstrapFragmentProvider::new(mode_echo_lister, mode_echo_loader);
    let ctx = FragmentContext {
        bootstrap_dir: "/unused".to_string(),
        bootstrap_mode: BootstrapMode::Full,
        session_role: SessionRole::Sub,
        ..FragmentContext::test_default()
    };
    let fragment = provider.generate(&ctx).await.unwrap();
    assert_eq!(fragment.content, "## AGENTS.md\nmode=Minimal");
}

/// Main agent with Full mode → lister and loader are called with Full.
#[tokio::test]
async fn test_main_role_passes_full_mode_to_injected_fns() {
    let provider = BootstrapFragmentProvider::new(mode_echo_lister, mode_echo_loader);
    let ctx = FragmentContext {
        bootstrap_dir: "/unused".to_string(),
        bootstrap_mode: BootstrapMode::Full,
        session_role: SessionRole::Main,
        ..FragmentContext::test_default()
    };
    let fragment = provider.generate(&ctx).await.unwrap();
    assert_eq!(fragment.content, "## BOOTSTRAP.md\nmode=Full");
}
