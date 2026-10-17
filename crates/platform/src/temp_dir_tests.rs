use crate::temp_dir::system_temp_dir;

// ── system_temp_dir: normal path (absolute path) ─────────────────

/// Doc promises "returns the absolute path of the system temporary directory".
#[test]
fn test_system_temp_dir_is_absolute() {
    let path = system_temp_dir();
    assert!(
        path.is_absolute(),
        "system_temp_dir() must return an absolute path, got: {path:?}"
    );
}

// ── system_temp_dir: consistency with std ────────────────────────

/// Doc promises "resolution basis identical to `std::env::temp_dir()`".
#[test]
fn test_system_temp_dir_matches_std_env_temp_dir() {
    let ours = system_temp_dir();
    let std_path = std::env::temp_dir();
    assert_eq!(
        ours, std_path,
        "system_temp_dir() must resolve identically to std::env::temp_dir()"
    );
}

// ── system_temp_dir: stability (stateless pure query) ────────────

/// Doc promises the interface is a pure path query: no creation, no cleanup,
/// no state — repeated calls must agree.
#[test]
fn test_system_temp_dir_deterministic() {
    let a = system_temp_dir();
    let b = system_temp_dir();
    assert_eq!(a, b, "system_temp_dir() must be deterministic across calls");
}

#[test]
fn test_system_temp_dir_deterministic_after_std_call() {
    let ours = system_temp_dir();
    // Interleave a std call: a stateless wrapper must stay in sync.
    let _ = std::env::temp_dir();
    let ours_again = system_temp_dir();
    assert_eq!(
        ours, ours_again,
        "system_temp_dir() must stay consistent when interleaved with std::env::temp_dir()"
    );
}
