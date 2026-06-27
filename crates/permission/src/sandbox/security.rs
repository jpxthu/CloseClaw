//! OS-level security policy for the sandboxed engine.

use std::path::PathBuf;

// ---------------------------------------------------------------------------
// Security Policy
// ---------------------------------------------------------------------------

/// Security policies applied to the engine subprocess.
///
/// On Linux, seccomp and landlock are used to restrict the engine's capabilities.
/// On non-Linux platforms, these are no-ops.
#[derive(Debug, Clone, Default)]
pub struct SecurityPolicy {
    /// Enable seccomp to restrict syscalls.
    pub seccomp: bool,
    /// Enable landlock to restrict filesystem access.
    pub landlock: bool,
    /// Explicitly allowed filesystem paths (used with landlock).
    pub allowed_fs_paths: Vec<PathBuf>,
    /// Explicitly blocked syscalls (used with seccomp).
    pub blocked_syscalls: Vec<String>,
}

impl SecurityPolicy {
    /// Create a default security policy that enables seccomp and landlock on Linux.
    pub fn default_restrictive() -> Self {
        Self {
            seccomp: cfg!(target_os = "linux"),
            landlock: cfg!(target_os = "linux"),
            allowed_fs_paths: vec![],
            blocked_syscalls: vec![],
        }
    }

    /// Apply the security policy inside the engine subprocess.
    ///
    /// Call this **once** at startup, before serving any requests.
    #[cfg(target_os = "linux")]
    pub fn apply(&self) -> anyhow::Result<()> {
        if self.seccomp {
            apply_seccomp()?;
        }
        if self.landlock {
            apply_landlock(&self.allowed_fs_paths)?;
        }
        Ok(())
    }

    #[cfg(not(target_os = "linux"))]
    pub fn apply(&self) -> anyhow::Result<()> {
        Ok(())
    }
}

#[cfg(target_os = "linux")]
fn apply_seccomp() -> anyhow::Result<()> {
    // seccomp enforcement is not yet implemented.
    // In production, use libseccomp or a BPF program via seccomp(2)
    // with SECCOMP_SET_MODE_FILTER.
    tracing::warn!(
        "SecurityPolicy::apply(): seccomp enforcement is a stub. \
         Kernel-level syscall filtering is NOT active."
    );
    Ok(())
}

#[cfg(target_os = "linux")]
fn apply_landlock(_allowed_paths: &[PathBuf]) -> anyhow::Result<()> {
    // Landlock enforcement is not yet implemented.
    // Landlock is available since Linux 5.13.
    // In production, call `landlock_create_ruleset()` and `landlock_add_rule()`
    // via the userspace ABI.
    tracing::warn!(
        "SecurityPolicy::apply(): landlock enforcement is a stub. \
         Filesystem sandboxing is NOT active."
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_security_policy_default_is_empty() {
        let policy = SecurityPolicy::default();
        assert!(!policy.seccomp, "default seccomp should be false");
        assert!(!policy.landlock, "default landlock should be false");
        assert!(
            policy.allowed_fs_paths.is_empty(),
            "default allowed_fs_paths should be empty"
        );
        assert!(
            policy.blocked_syscalls.is_empty(),
            "default blocked_syscalls should be empty"
        );
    }

    #[test]
    fn test_security_policy_apply_returns_ok() {
        let policy = SecurityPolicy::default();
        // apply() should return Ok on any platform (no-op on non-Linux, stub on Linux)
        let result = policy.apply();
        assert!(result.is_ok(), "apply() should succeed, got: {:?}", result);
    }

    #[test]
    fn test_security_policy_custom() {
        let policy = SecurityPolicy {
            seccomp: false,
            landlock: false,
            allowed_fs_paths: vec![std::path::PathBuf::from("/tmp/custom")],
            blocked_syscalls: vec!["read".to_string(), "write".to_string()],
        };
        assert!(!policy.seccomp);
        assert!(!policy.landlock);
        assert_eq!(policy.allowed_fs_paths.len(), 1);
        assert_eq!(policy.blocked_syscalls.len(), 2);
        let result = policy.apply();
        assert!(result.is_ok());
    }

    #[test]
    fn test_default_restrictive_linux() {
        let policy = SecurityPolicy::default_restrictive();
        if cfg!(target_os = "linux") {
            assert!(policy.seccomp, "seccomp should be enabled on Linux");
            assert!(policy.landlock, "landlock should be enabled on Linux");
        } else {
            assert!(!policy.seccomp, "seccomp should be disabled on non-Linux");
            assert!(!policy.landlock, "landlock should be disabled on non-Linux");
        }
    }

    #[test]
    fn test_default_restrictive_has_empty_paths() {
        let policy = SecurityPolicy::default_restrictive();
        assert!(
            policy.allowed_fs_paths.is_empty(),
            "allowed_fs_paths should be empty"
        );
        assert!(
            policy.blocked_syscalls.is_empty(),
            "blocked_syscalls should be empty"
        );
    }
}
