//! Unit tests for `test_helpers` — `ensure_dir`'s TempDir contract and the
//! shared `MemAuditLogger` audit mock behaviour (filter, time-descending
//! query, snapshot isolation, empty/zero-match boundaries, concurrency).

use std::sync::Arc;

use crate::audit_log::{AuditDisposition, AuditLogEntry, AuditLogFilter, AuditLogger};
use crate::permission_types::RiskLevel;
use crate::test_helpers::{ensure_dir, MemAuditLogger};

fn make_entry(agent: &str, ts: &str, disp: AuditDisposition) -> AuditLogEntry {
    AuditLogEntry {
        timestamp: ts.to_string(),
        agent_id: agent.to_string(),
        tool_name: "file".to_string(),
        operation: "write /x".to_string(),
        reason: "test".to_string(),
        risk_level: RiskLevel::Low,
        session_mode: None,
        disposition: disp,
    }
}

#[test]
fn test_ensure_dir_creates_subdir_under_tempdir() {
    let tmp = tempfile::TempDir::new().expect("temp dir");
    let subdir = tmp.path().join("nested/deeper");
    ensure_dir(&subdir).expect("create nested subdir under TempDir");
    assert!(subdir.is_dir(), "subdir should exist: {subdir:?}");
}

// The panic path under test comes from `debug_assert!` in `ensure_dir`,
// which is compiled out in release builds — this counter-example is only
// meaningful for the debug profile, so it is compiled (and run) solely
// when `debug_assertions` is on.
#[test]
#[cfg(debug_assertions)]
#[should_panic(expected = "ensure_dir")]
fn test_ensure_dir_panics_for_path_outside_tempdir() {
    let outside = std::env::temp_dir()
        .parent()
        .map(std::path::Path::to_path_buf)
        .unwrap_or_else(|| std::path::PathBuf::from("/proc"));
    ensure_dir(&outside.join("closeclaw_ensure_dir_outside_probe"))
        .expect("ensure_dir: create_dir_all outside temp dir must fail");
}

/// 正常路径：log 多条后 `query_entries` 经 `AuditLogFilter` 命中正确子集
/// ——agent_id 单维命中全部该 agent 条目；agent_id + disposition 双维
/// 只保留同时匹配的条目（时间戳逐一核对，非仅数量断言）。
#[test]
fn test_mem_audit_logger_query_filters_matching_subset() {
    let logger = MemAuditLogger::new();
    logger.log(&make_entry(
        "a1",
        "2026-01-01T00:00:00Z",
        AuditDisposition::Approved,
    ));
    logger.log(&make_entry(
        "a2",
        "2026-01-02T00:00:00Z",
        AuditDisposition::Approved,
    ));
    logger.log(&make_entry(
        "a1",
        "2026-01-03T00:00:00Z",
        AuditDisposition::Rejected,
    ));
    logger.log(&make_entry(
        "a1",
        "2026-01-04T00:00:00Z",
        AuditDisposition::Approved,
    ));
    logger.log(&make_entry(
        "a3",
        "2026-01-05T00:00:00Z",
        AuditDisposition::Rejected,
    ));

    let a1_only = logger.query_entries(&AuditLogFilter {
        agent_id: Some("a1".to_string()),
        ..Default::default()
    });
    assert_eq!(a1_only.len(), 3, "a1 共三条，必须全部命中");
    assert!(a1_only.iter().all(|e| e.agent_id == "a1"));

    let a1_approved = logger.query_entries(&AuditLogFilter {
        agent_id: Some("a1".to_string()),
        disposition: Some(AuditDisposition::Approved),
        ..Default::default()
    });
    assert_eq!(
        a1_approved.len(),
        2,
        "双维过滤必须只命中 a1 的 Approved 条目"
    );
    assert!(a1_approved
        .iter()
        .all(|e| e.agent_id == "a1" && e.disposition == AuditDisposition::Approved));
    let mut timestamps: Vec<&str> = a1_approved.iter().map(|e| e.timestamp.as_str()).collect();
    timestamps.sort_unstable();
    assert_eq!(
        timestamps,
        vec!["2026-01-01T00:00:00Z", "2026-01-04T00:00:00Z"],
        "命中的必须是正确子集（时间戳一一对应）"
    );
}

/// 时间倒序：乱序时间戳写入后，query 严格按 timestamp 降序返回
/// （相邻两条逐一核对严格递减）；对照 `entries()` 保持插入序，
/// 证明排序只发生在 query_entries 内。
#[test]
fn test_mem_audit_logger_query_sorts_time_descending() {
    let logger = MemAuditLogger::new();
    let shuffled = [
        "2026-01-03T00:00:00Z",
        "2026-01-01T00:00:00Z",
        "2026-01-05T00:00:00Z",
        "2026-01-02T00:00:00Z",
        "2026-01-04T00:00:00Z",
    ];
    for (i, ts) in shuffled.iter().enumerate() {
        logger.log(&make_entry(
            &format!("agent-{i}"),
            ts,
            AuditDisposition::Approved,
        ));
    }

    let queried = logger.query_entries(&AuditLogFilter::default());
    for pair in queried.windows(2) {
        assert!(
            pair[0].timestamp > pair[1].timestamp,
            "相邻两条必须严格递减：{} 应晚于 {}",
            pair[0].timestamp,
            pair[1].timestamp
        );
    }
    let mut expected = shuffled.to_vec();
    expected.sort_unstable();
    expected.reverse();
    let timestamps: Vec<&str> = queried.iter().map(|e| e.timestamp.as_str()).collect();
    assert_eq!(timestamps, expected, "query 结果必须严格按 timestamp 倒序");

    let inserted_entries = logger.entries();
    let inserted: Vec<&str> = inserted_entries
        .iter()
        .map(|e| e.timestamp.as_str())
        .collect();
    assert_eq!(
        inserted,
        shuffled.to_vec(),
        "entries() 必须保持插入序（排序只属于 query 语义）"
    );
}

/// 混合时区形态：`+08:00` / `Z` / `+00:00` 乱序写入后，query 按解析后的
/// 真实时刻降序返回（与 `AuditLogFilter::matches` 时间窗同口径）——
/// 字符串字典序会把 `08:00+08:00`（= `00:00Z`）排到 `01:00Z` 之前而误判。
#[test]
fn test_mem_audit_logger_query_sorts_mixed_timezone_forms_by_instant() {
    let logger = MemAuditLogger::new();
    let inserted = [
        ("a-plus8-02z", "2026-01-02T10:00:00+08:00"),
        ("a-z-01z", "2026-01-02T01:00:00Z"),
        ("a-plus8-00z", "2026-01-02T08:00:00+08:00"),
        ("a-z00-03z", "2026-01-02T03:00:00+00:00"),
        ("a-z-prev-day", "2026-01-01T23:00:00Z"),
    ];
    for (agent, ts) in inserted {
        logger.log(&make_entry(agent, ts, AuditDisposition::Approved));
    }

    let queried = logger.query_entries(&AuditLogFilter::default());
    let agents: Vec<&str> = queried.iter().map(|e| e.agent_id.as_str()).collect();
    assert_eq!(
        agents,
        vec![
            "a-z00-03z",    // 03:00Z
            "a-plus8-02z",  // 02:00Z（10:00+08:00）
            "a-z-01z",      // 01:00Z
            "a-plus8-00z",  // 00:00Z（08:00+08:00）
            "a-z-prev-day", // 前一日 23:00Z
        ],
        "query 必须按解析后的真实时刻降序，而非 timestamp 字符串字典序"
    );
}

/// 快照语义：`entries()` 返回全量快照且与后续写入隔离——拍快照后
/// 继续 log，快照不变；修改快照 Vec 也不得穿透到 logger 内部状态。
#[test]
fn test_mem_audit_logger_entries_snapshot_isolated_from_later_writes() {
    let logger = MemAuditLogger::new();
    logger.log(&make_entry(
        "a1",
        "2026-01-01T00:00:00Z",
        AuditDisposition::Approved,
    ));
    logger.log(&make_entry(
        "a2",
        "2026-01-02T00:00:00Z",
        AuditDisposition::Rejected,
    ));

    let snapshot = logger.entries();
    assert_eq!(snapshot.len(), 2, "快照必须包含全部已写入条目");
    let agents: Vec<&str> = snapshot.iter().map(|e| e.agent_id.as_str()).collect();
    assert_eq!(agents, vec!["a1", "a2"], "快照按插入序包含全部条目");

    logger.log(&make_entry(
        "a3",
        "2026-01-03T00:00:00Z",
        AuditDisposition::Approved,
    ));
    assert_eq!(snapshot.len(), 2, "后续写入不得改变已取得的快照");

    let mut mutable = logger.entries();
    mutable.clear();
    let fresh = logger.entries();
    assert_eq!(fresh.len(), 3, "修改快照 Vec 不得穿透到 logger 内部");
    let fresh_agents: Vec<&str> = fresh.iter().map(|e| e.agent_id.as_str()).collect();
    assert_eq!(fresh_agents, vec!["a1", "a2", "a3"]);
}

/// 边界：空 logger 的 query / entries 都返回空 Vec；有内容后，匹配
/// 零条的 filter（不存在的 agent、空时间窗）也返回空 Vec，并以一条
/// 恰好命中时间窗的对照断言证明上一断言并非 filter 失效。
#[test]
fn test_mem_audit_logger_empty_and_zero_match_queries() {
    let empty = MemAuditLogger::new();
    assert!(
        empty.query_entries(&AuditLogFilter::default()).is_empty(),
        "空 logger 的 query 必须返回空 Vec"
    );
    assert!(
        empty.entries().is_empty(),
        "空 logger 的 entries 必须返回空 Vec"
    );

    let logger = MemAuditLogger::new();
    logger.log(&make_entry(
        "a1",
        "2026-01-02T00:00:00Z",
        AuditDisposition::Approved,
    ));

    let no_agent = logger.query_entries(&AuditLogFilter {
        agent_id: Some("missing".to_string()),
        ..Default::default()
    });
    assert!(no_agent.is_empty(), "不存在的 agent 必须零命中");

    let no_window = logger.query_entries(&AuditLogFilter {
        since: Some("2026-01-03T00:00:00Z".to_string()),
        until: Some("2026-01-04T00:00:00Z".to_string()),
        ..Default::default()
    });
    assert!(no_window.is_empty(), "时间窗外必须零命中");

    let hit = logger.query_entries(&AuditLogFilter {
        since: Some("2026-01-02T00:00:00Z".to_string()),
        until: Some("2026-01-02T00:00:00Z".to_string()),
        ..Default::default()
    });
    assert_eq!(hit.len(), 1, "对照：时间窗恰好命中（含端点）时非空");
}

/// 状态/并发：`Arc<dyn AuditLogger>` 跨 8 线程各写 50 条，总数精确
/// 等于写入数（无丢条）、每 agent 恰好 50 条（无重复）——验证
/// trait 的 Send + Sync 契约与 Mutex 状态一致性；query 路径同样全量可见。
#[test]
fn test_mem_audit_logger_concurrent_log_no_lost_entries() {
    const THREADS: usize = 8;
    const PER_THREAD: usize = 50;

    let concrete = Arc::new(MemAuditLogger::new());
    let logger: Arc<dyn AuditLogger> = Arc::clone(&concrete) as Arc<dyn AuditLogger>;
    let handles: Vec<_> = (0..THREADS)
        .map(|t| {
            let logger = Arc::clone(&logger);
            std::thread::spawn(move || {
                for i in 0..PER_THREAD {
                    let disp = if i % 2 == 0 {
                        AuditDisposition::Approved
                    } else {
                        AuditDisposition::Rejected
                    };
                    logger.log(&make_entry(
                        &format!("agent-{t}"),
                        &format!("2026-01-01T00:00:{i:02}Z"),
                        disp,
                    ));
                }
            })
        })
        .collect();
    for handle in handles {
        handle.join().expect("writer thread must not panic");
    }

    let all = concrete.entries();
    assert_eq!(
        all.len(),
        THREADS * PER_THREAD,
        "并发写入不得丢条：总数必须精确等于写入数"
    );
    for t in 0..THREADS {
        let count = all
            .iter()
            .filter(|e| e.agent_id == format!("agent-{t}"))
            .count();
        assert_eq!(count, PER_THREAD, "agent-{t} 应恰好写入 {PER_THREAD} 条");
    }

    let queried = logger.query_entries(&AuditLogFilter::default());
    assert_eq!(
        queried.len(),
        THREADS * PER_THREAD,
        "经 trait 的 query 路径必须同样全量可见"
    );
}
