use super::*;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;
use tempfile::TempDir;

#[tokio::test]
async fn test_same_file_serializes() {
    let map = Arc::new(FileMutexMap::new());
    let dir = TempDir::new().unwrap();
    let file = dir.path().join("a.txt");
    std::fs::write(&file, "").unwrap();

    let counter = Arc::new(AtomicUsize::new(0));
    let c1 = counter.clone();
    let c2 = counter.clone();
    let f1 = file.clone();
    let f2 = file.clone();
    let map2 = map.clone();
    let map3 = map.clone();

    let h1 = tokio::spawn(async move {
        let g = map2.acquire(&f1).await;
        c1.fetch_add(1, Ordering::SeqCst);
        tokio::time::sleep(Duration::from_millis(50)).await;
        let val = c1.load(Ordering::SeqCst);
        drop(g);
        val
    });

    // Small delay so h1 acquires first.
    tokio::time::sleep(Duration::from_millis(10)).await;

    let h2 = tokio::spawn(async move {
        let g = map3.acquire(&f2).await;
        c2.fetch_add(1, Ordering::SeqCst);
        let val = c2.load(Ordering::SeqCst);
        drop(g);
        val
    });

    let v1 = h1.await.unwrap();
    let v2 = h2.await.unwrap();

    // h2 must see counter == 2 (h1 already incremented) because both target
    // the same file and should serialize.
    assert_eq!(v1, 1);
    assert_eq!(v2, 2);
}

#[tokio::test]
async fn test_different_files_parallel() {
    let map = Arc::new(FileMutexMap::new());
    let dir = TempDir::new().unwrap();
    let f1 = dir.path().join("a.txt");
    let f2 = dir.path().join("b.txt");
    std::fs::write(&f1, "").unwrap();
    std::fs::write(&f2, "").unwrap();

    let started = Arc::new(AtomicUsize::new(0));
    let s1 = started.clone();
    let s2 = started.clone();
    let map2 = map.clone();
    let map3 = map.clone();

    let h1 = tokio::spawn(async move {
        let _g = map2.acquire(&f1).await;
        s1.fetch_add(1, Ordering::SeqCst);
        tokio::time::sleep(Duration::from_millis(50)).await;
        drop(_g);
    });

    let h2 = tokio::spawn(async move {
        let _g = map3.acquire(&f2).await;
        s2.fetch_add(1, Ordering::SeqCst);
        tokio::time::sleep(Duration::from_millis(50)).await;
        drop(_g);
    });

    // Both should start quickly (different files, no contention).
    tokio::time::sleep(Duration::from_millis(20)).await;
    assert_eq!(started.load(Ordering::SeqCst), 2);

    h1.await.unwrap();
    h2.await.unwrap();
}

#[tokio::test]
async fn test_canonicalize_symlink() {
    let map = FileMutexMap::new();
    let dir = TempDir::new().unwrap();
    let real = dir.path().join("real.txt");
    let link = dir.path().join("link.txt");
    std::fs::write(&real, "data").unwrap();
    std::os::unix::fs::symlink(&real, &link).unwrap();

    // Acquire via symlink path — should resolve to the same mutex as the real path.
    let g1 = map.acquire(&link).await;
    // try_acquire via real path should see it as held.
    match map.try_acquire(&real) {
        TryAcquireResult::WouldBlock => {}
        _ => panic!("expected WouldBlock for same canonical path"),
    }
    drop(g1);

    // Now it should be free.
    match map.try_acquire(&real) {
        TryAcquireResult::Acquired(_g) => {}
        _ => panic!("expected Acquired after drop"),
    }
}

#[tokio::test]
async fn test_drop_reacquire() {
    let map = FileMutexMap::new();
    let dir = TempDir::new().unwrap();
    let file = dir.path().join("test.txt");
    std::fs::write(&file, "").unwrap();

    let g = map.acquire(&file).await;
    drop(g);

    // Should be re-acquirable immediately.
    match map.try_acquire(&file) {
        TryAcquireResult::Acquired(_g) => {}
        _ => panic!("expected Acquired after drop"),
    }
}

#[tokio::test]
async fn test_cleanup_removes_unused_entry() {
    let map = FileMutexMap::new();
    let dir = TempDir::new().unwrap();
    let file = dir.path().join("cleanup.txt");
    std::fs::write(&file, "").unwrap();

    {
        let _g = map.acquire(&file).await;
        assert_eq!(map.inner.len(), 1);
        // _g still live — cleanup should NOT remove the entry.
        map.cleanup(&file);
        assert_eq!(map.inner.len(), 1);
    }
    // After block, _g is dropped — only the DashMap ref remains.
    map.cleanup(&file);
    assert_eq!(map.inner.len(), 0);
}

#[tokio::test]
async fn test_try_acquire_would_block() {
    let map = FileMutexMap::new();
    let dir = TempDir::new().unwrap();
    let file = dir.path().join("block.txt");
    std::fs::write(&file, "").unwrap();

    let _g = map.acquire(&file).await;

    match map.try_acquire(&file) {
        TryAcquireResult::WouldBlock => {}
        _ => panic!("expected WouldBlock"),
    }
}
