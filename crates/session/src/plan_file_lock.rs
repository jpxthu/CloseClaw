//! Locked and atomic write primitives for plan files.
//!
//! Owns the plan-file locking and durable-write mechanism: the sidecar
//! `{plan}.lock` exclusive advisory lock ([`acquire_plan_lock`],
//! [`lock_file_exclusive`]), the tempfile + fsync + rename atomic write
//! ([`write_atomically`]), and the two writers built on them — the
//! full-file [`write_plan_file_locked`] and the read-modify-write
//! [`mutate_plan_file`]. Session-side helpers, the Write/Edit tools (via
//! their port), and the archival rename all share this single lock domain.

use std::fs::File;
use std::io::{self, Write};
use std::os::fd::AsRawFd;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use uuid::Uuid;

// ── Atomic write ─────────────────────────────────────────────────────────

/// Write content to a file atomically: write to a temp file, fsync,
/// then rename to the target path. Cleans up the temp file on failure.
///
/// If `file_permissions` is `Some(mode)`, the target file's permission
/// is set to `mode` (octal, e.g. `0o600`) after the rename.
pub(crate) fn write_atomically(
    path: &Path,
    content: &[u8],
    file_permissions: Option<u32>,
) -> io::Result<()> {
    let parent = path.parent().unwrap_or(Path::new("."));
    std::fs::create_dir_all(parent)?;

    let tmp_name = format!(".tmp.{}", Uuid::new_v4());
    let tmp_path = parent.join(tmp_name);

    let mut file = File::create(&tmp_path)?;

    if let Err(e) = file.write_all(content) {
        let _ = std::fs::remove_file(&tmp_path);
        return Err(e);
    }

    if let Err(e) = file.sync_all() {
        let _ = std::fs::remove_file(&tmp_path);
        return Err(e);
    }

    // Sync parent directory to make the rename durable.
    if let Ok(dir) = File::open(parent) {
        let _ = dir.sync_all();
    }

    if let Err(e) = std::fs::rename(&tmp_path, path) {
        let _ = std::fs::remove_file(&tmp_path);
        return Err(e);
    }

    // Set file permissions after rename to ensure the final file has
    // the correct mode. This is done post-rename so the rename itself
    // remains atomic.
    if let Some(mode) = file_permissions {
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(mode))?;
    }

    Ok(())
}

// ── Locked atomic write ──────────────────────────────────────────────────

/// Write `content` to a plan file atomically while holding its exclusive
/// sidecar lock.
///
/// This is the public write entry for out-of-crate writers (e.g. the
/// Write/Edit tools through their port) that must share the same lock
/// domain as the session-side read-modify-write helpers
/// ([`touch_access_timestamp`](crate::plan_file::touch_access_timestamp),
/// [`append_to_plan_section`](crate::plan_file::append_to_plan_section)) — without it,
/// concurrent writes from two lock domains can interleave and permanently
/// lose sections.
///
/// The whole operation — parent-dir creation → lock acquisition → mode read
/// → atomic rename — runs while holding an exclusive advisory lock on the
/// sidecar `{plan_path}.lock`, the same lock domain as `mutate_plan_file`
/// and the archival rename. If the target file already exists, its permission
/// mode is preserved across the atomic replace; if it does not exist, missing
/// parent directories are created and the file receives the default mode.
///
/// # Reentrancy
///
/// Like `acquire_plan_lock`, the lock is **not** reentrant: calling this
/// while the same plan's sidecar lock is already held (this thread or
/// another descriptor in this process) blocks forever. Callers must never
/// invoke this while holding the same plan's lock.
///
/// # Errors
/// Returns the first error from creating the parent directories, creating/
/// locking the sidecar, reading the existing file's metadata, or the atomic
/// write itself.
pub fn write_plan_file_locked(plan_path: &Path, content: &str) -> io::Result<()> {
    // The sidecar lives next to the target, so its parent must exist before
    // the lock can be taken. `create_dir_all` is idempotent; the lock below
    // still serializes every writer of the same target.
    if let Some(parent) = plan_path.parent() {
        std::fs::create_dir_all(parent)?;
    }

    let _lock = acquire_plan_lock(plan_path)?;

    // Single stat instead of `exists()` + `metadata()`: the NotFound branch
    // is the "new file" case (default mode), any other metadata error is
    // propagated. One syscall narrows the window in which the target could
    // appear or disappear between check and use.
    match std::fs::metadata(plan_path) {
        Ok(meta) => {
            let mode = meta.permissions().mode();
            write_atomically(plan_path, content.as_bytes(), Some(mode))
        }
        Err(e) if e.kind() == io::ErrorKind::NotFound => {
            write_atomically(plan_path, content.as_bytes(), None)
        }
        Err(e) => Err(e),
    }
}

// ── Concurrency-safe read-modify-write ─────────────────────────────────

/// Perform a read-modify-write cycle on a plan file under an exclusive lock.
///
/// The whole cycle — read → `transform` → atomic write-back — runs while
/// holding an exclusive advisory lock on the sidecar `{path}.lock` file, so
/// concurrent writers (threads and separate processes alike) are serialized
/// and can never observe a truncated file or lose each other's updates. The
/// write-back goes through [`write_atomically`] (tempfile +
/// fsync + rename) with the original file mode preserved, so readers only
/// ever see the complete old content or the complete new content.
///
/// The sidecar's `lock` extension never matches the `.md` filter used by plan
/// resolution, listing, and archiving, so lock files stay invisible to them.
/// No global mutable state is involved: exclusion is scoped to the sidecar
/// file only.
///
/// # Reentrancy
///
/// `transform` runs while the sidecar lock is held and must **not** call back
/// into any lock-taking plan API ([`touch_access_timestamp`],
/// [`update_plan_timestamp`],
/// [`append_to_plan_section`], [`acquire_plan_lock`], or the
/// archival rename in `plan_archive`). Every
/// acquisition locks a fresh file descriptor, so a same-process re-lock of
/// the same sidecar blocks forever (flock treats descriptors independently —
/// self-deadlock, no error is ever returned).
///
/// # Errors
/// Returns `NotFound` if the plan file does not exist — checked while the
/// lock is held, so the check cannot race an archival rename — otherwise the
/// first error from creating/locking the sidecar, reading the plan file,
/// running `transform`, or writing the result back.
pub(crate) fn mutate_plan_file<F>(plan_path: &Path, transform: F) -> io::Result<()>
where
    F: FnOnce(&mut String) -> io::Result<()>,
{
    let _lock = acquire_plan_lock(plan_path)?;

    // Checked under the lock: a pre-lock check would race a concurrent
    // archival rename (check-then-act) and split NotFound across two sources.
    if !plan_path.exists() {
        return Err(io::Error::new(
            io::ErrorKind::NotFound,
            format!("plan file not found: {}", plan_path.display()),
        ));
    }

    let mut content = std::fs::read_to_string(plan_path)?;
    transform(&mut content)?;

    let mode = std::fs::metadata(plan_path)?.permissions().mode();
    write_atomically(plan_path, content.as_bytes(), Some(mode))
}

/// Take a blocking exclusive advisory lock on `file`'s descriptor.
///
/// MSRV-compatible equivalent of [`lock`] (stabilized in Rust
/// 1.89): a single `flock(LOCK_EX)` call, with the resulting `io::Error`
/// propagated as-is — `EINTR` is **not** retried and surfaces as
/// `io::ErrorKind::Interrupted`, exactly like the std Unix implementation.
/// The lock is advisory, scoped to the open file description behind the
/// descriptor (independent descriptors — same process or another — contend
/// for it), and released by the kernel when the descriptor is closed.
fn lock_file_exclusive(file: &File) -> io::Result<()> {
    // SAFETY: `file` is a valid open `File` whose descriptor stays alive
    // for the whole call; `flock` only updates kernel lock state for that
    // descriptor — it dereferences no user memory and cannot unwind.
    let rc = unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX) };
    if rc == 0 {
        Ok(())
    } else {
        Err(io::Error::last_os_error())
    }
}

/// Open (creating it if needed) the sidecar lock file for `plan_path` and
/// take the exclusive advisory lock on it.
///
/// The sidecar lives at `{plan_path}.lock`; the caller holds the lock until
/// the returned [`File`] is dropped. Its `lock` extension never matches the
/// `.md` filter used by plan resolution, listing, and archiving, so lock
/// files stay invisible to them.
///
/// This is the single lock domain shared by every plan-file writer — the
/// [`mutate_plan_file`] read-modify-write helpers, [`write_plan_file_locked`],
/// and the archival rename in `plan_archive` alike. It involves no global
/// mutable state.
///
/// # Reentrancy
///
/// The lock is **not** reentrant. Every call opens a fresh sidecar descriptor,
/// so acquiring it again for the same plan while it is already held — by this
/// thread, another thread, or another handle in this process — blocks forever
/// (flock is per-descriptor: self-deadlock, not an error). Callers must never
/// invoke this, [`mutate_plan_file`], or the archival rename while the same
/// plan's lock is already held (e.g. from inside a `transform` closure).
///
/// # Errors
/// Returns an error if the sidecar cannot be created or locked.
pub(crate) fn acquire_plan_lock(plan_path: &Path) -> io::Result<File> {
    let mut lock_name = plan_path.as_os_str().to_os_string();
    lock_name.push(".lock");
    let lock_file = File::create(PathBuf::from(&lock_name))?;
    lock_file_exclusive(&lock_file)?;
    Ok(lock_file)
}
