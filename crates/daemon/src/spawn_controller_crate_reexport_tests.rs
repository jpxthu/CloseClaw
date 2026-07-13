//! Step 1.5 — crate归属 test: SpawnController accessible via daemon crate.
//!
//! The design doc (`agent-spawn.md`) places SpawnController at the daemon layer.
//! This test verifies the re-export is functional.

/// Compile-time verification that SpawnController is accessible via the
/// `closeclaw_daemon::SpawnController` path, as required by the design doc
/// which places SpawnController at the daemon layer.
///
/// This is a type-level check: if the re-export is broken, this won't compile.
#[test]
fn test_crate_daemon_reexport() {
    // This import proves closeclaw_daemon::SpawnController is accessible.
    // If the re-export in daemon's mod.rs is broken, this line won't compile.
    let _type_check: fn() -> crate::SpawnController = || unreachable!();
}
