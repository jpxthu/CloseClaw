//! `test_helpers` 的单元测试——覆盖 `ensure_dir` 的 TempDir 契约与
//! 父路径为普通文件的错误路径、`remove_subdir` 的递归摘除 TempDir
//! 契约、`create_subdir` 的嵌套路径创建、TempDir 契约与 base 为普通
//! 文件的错误路径、`advance_mtime` 的 mtime 推进 / 零推进 / 路径缺失 /
//! 重复推进 / TempDir 契约外行为，以及共享的 `MemAuditLogger` 审计
//! mock 行为（过滤、时间降序查询、不可解析时间戳排序、时间窗与
//! matches 一致性、快照隔离、空与零命中边界、并发）。

use std::sync::Arc;
use std::time::Duration;

use crate::audit_log::{AuditDisposition, AuditLogEntry, AuditLogFilter, AuditLogger};
use crate::test_helpers::{
    advance_mtime, create_subdir, ensure_dir, remove_subdir, sample_audit_entry, MemAuditLogger,
};

/// query 结果中 agent_id 的出现顺序（排序断言的公共取样口径）。
fn agent_order(entries: &[AuditLogEntry]) -> Vec<&str> {
    entries.iter().map(|e| e.agent_id.as_str()).collect()
}

/// 薄封装：写入一条 Approved 样例条目——字段构造点保持在
/// `sample_audit_entry` 唯一。
fn log_approved(logger: &MemAuditLogger, agent: &str, ts: &str) {
    logger.log(&sample_audit_entry(agent, ts, AuditDisposition::Approved));
}

/// 正常路径：`ensure_dir` 在 TempDir 下创建嵌套相对路径，递归补齐
/// 缺失的父级目录，创建后目标路径真实存在。
#[test]
fn test_ensure_dir_creates_subdir_under_tempdir() {
    let tmp = tempfile::TempDir::new().expect("temp dir");
    let subdir = tmp.path().join("nested/deeper");
    ensure_dir(&subdir).expect("create nested subdir under TempDir");
    assert!(subdir.is_dir(), "创建后子目录应真实存在：{subdir:?}");
}

// 被测的 panic 路径来自 `ensure_dir` / `remove_subdir` /
// `create_subdir` 内的 `debug_assert!`，该断言在 release 构建下被编译
// 剔除——这些反例只对 debug profile 有意义，因此仅在开启
// `debug_assertions` 时编译（并运行）。探测路径取 `std::env::temp_dir()`
// 的词法兄弟路径（其 parent + `tail`），必然落在 TempDir 契约之外；
// 退化场景（无父目录的 `temp_dir()`）不存在这样的兄弟路径，会直接经
// `expect` 失败报错，而不是回退到硬编码的环境路径——那样的契约将
// 依赖文件系统布局。
#[cfg(debug_assertions)]
fn outside_tempdir_probe(tail: &str) -> std::path::PathBuf {
    std::env::temp_dir()
        .parent()
        .expect("outside-tempdir probe requires a parented temp_dir root")
        .join(tail)
}

/// TempDir 契约：debug 构建下路径落在 `std::env::temp_dir()` 之外时
/// 在任何文件系统写入前快速失败（panic 前缀与 helper 名一致），与
/// `create_subdir` / `advance_mtime` 的契约口径对齐。
#[test]
#[cfg(debug_assertions)]
#[should_panic(expected = "ensure_dir")]
fn test_ensure_dir_panics_for_path_outside_tempdir() {
    let outside = outside_tempdir_probe("closeclaw_ensure_dir_outside_probe");
    ensure_dir(&outside).expect("ensure_dir: directory creation outside temp dir must fail");
}

/// 错误路径：目标路径的父组件是普通文件时目录创建返回 Err，
/// `ensure_dir` 以 `io::Result` 原样透传（`NotADirectory`）而不 panic，
/// 错误处理权留给调用者；目标目录不得被创建。
#[test]
fn test_ensure_dir_returns_error_when_parent_is_a_regular_file() {
    let tmp = tempfile::TempDir::new().expect("temp dir");
    let blocker = tmp.path().join("blocker.txt");
    std::fs::write(&blocker, "x").expect("write blocker file");
    let dir = blocker.join("child");

    let err = ensure_dir(&dir).expect_err("父路径为普通文件时创建目录必须失败");
    assert_eq!(
        err.kind(),
        std::io::ErrorKind::NotADirectory,
        "错误种类必须是 NotADirectory，实际：{err:?}"
    );
    assert!(!dir.exists(), "失败的创建不得留下目标目录：{dir:?}");
}

/// 正常路径：`remove_subdir` 递归删除 TempDir 下含文件的目录树，
/// 删除后目录不存在；TempDir 根本身保持存在（只摘除子树）。
#[test]
fn test_remove_subdir_deletes_subtree_under_tempdir() {
    let tmp = tempfile::TempDir::new().expect("temp dir");
    let subdir = tmp.path().join("skill-b");
    ensure_dir(&subdir).expect("create subdir under TempDir");
    std::fs::write(subdir.join("SKILL.md"), "x").expect("write file");

    remove_subdir(&subdir).expect("remove subdir under TempDir");
    assert!(!subdir.exists(), "子树应被递归删除：{subdir:?}");
    assert!(tmp.path().is_dir(), "TempDir 根应保持存在");
}

/// 幂等边界由调用方决定（目录可能本就不存在）：`remove_subdir` 对缺失
/// 目录返回错误而非 panic，错误处理权留给调用者（共享版额外
/// debug-assert TempDir 契约）。
#[test]
fn test_remove_subdir_missing_dir_returns_error() {
    let tmp = tempfile::TempDir::new().expect("temp dir");
    let missing = tmp.path().join("never-created");
    let result = remove_subdir(&missing);
    assert!(result.is_err(), "缺失目录应返回错误而非 panic");
}

/// TempDir 契约：debug 构建下路径落在 `std::env::temp_dir()` 之外时
/// 在任何删除操作前快速失败（panic 前缀与 helper 名一致），与
/// `ensure_dir` / `create_subdir` 的契约口径对齐。
#[test]
#[cfg(debug_assertions)]
#[should_panic(expected = "remove_subdir")]
fn test_remove_subdir_panics_for_path_outside_tempdir() {
    let outside = outside_tempdir_probe("closeclaw_remove_subdir_outside_probe");
    remove_subdir(&outside).expect("remove_subdir: deletion outside temp dir must fail");
}

/// 正常路径：`create_subdir` 在 TempDir 基目录下创建嵌套相对路径，
/// 返回值恰为 `base.join(rel)` 且目录真实存在。
#[test]
fn test_create_subdir_creates_nested_path_under_tempdir() {
    let tmp = tempfile::TempDir::new().expect("temp dir");
    let created = create_subdir(tmp.path(), "a/b/c");
    assert_eq!(
        created,
        tmp.path().join("a/b/c"),
        "返回路径必须等于 base.join(rel)"
    );
    assert!(created.is_dir(), "嵌套子目录应真实存在：{created:?}");
}

/// 幂等：对已存在的目录再次调用不报错、返回相同路径（创建语义幂等）。
#[test]
fn test_create_subdir_existing_dir_is_idempotent() {
    let tmp = tempfile::TempDir::new().expect("temp dir");
    let first = create_subdir(tmp.path(), "a/b/c");
    let second = create_subdir(tmp.path(), "a/b/c");
    assert_eq!(first, second, "重复调用必须返回相同路径");
    assert!(second.is_dir(), "目录应保持存在：{second:?}");
}

/// TempDir 契约：debug 构建下 base 落在 `std::env::temp_dir()` 之外时
/// 在任何文件系统写入前快速失败（panic 前缀与 helper 名一致），
/// 与 `ensure_dir` / `remove_subdir` 的契约口径对齐。
#[test]
#[cfg(debug_assertions)]
#[should_panic(expected = "create_subdir")]
fn test_create_subdir_panics_for_base_outside_tempdir() {
    let outside = outside_tempdir_probe("closeclaw_create_subdir_outside_probe");
    create_subdir(&outside, "rel");
}

/// 错误路径：base 是普通文件时（TempDir 之下，TempDir 契约本身放行）
/// 目录创建必然失败（ENOTDIR），unwrap 型契约让失败以 panic 浮出，
/// 错误不得被静默吞掉；panic 消息含错误种类 `NotADirectory`，证明
/// 走的是 unwrap-on-Err 而非 TempDir 契约 panic。
#[test]
#[should_panic(expected = "NotADirectory")]
fn test_create_subdir_panics_when_base_is_a_regular_file() {
    let tmp = tempfile::TempDir::new().expect("temp dir");
    let blocker = tmp.path().join("blocker.txt");
    std::fs::write(&blocker, "x").expect("write blocker file");
    create_subdir(&blocker, "child");
}

/// 正常路径：写入文件后推进 10s，新 mtime 必须 ≥ 原 mtime + 10s − ε
/// （ε = 1s，容忍文件系统时间戳粒度截断）且严格晚于原值，保证缓存失效
/// 判定（比较文件 mtime 与先前记录值）必然把文件视为已更新。
#[test]
fn test_advance_mtime_advances_file_mtime_by_ten_seconds() {
    const SECS: u64 = 10;
    const EPSILON: Duration = Duration::from_secs(1);

    let tmp = tempfile::TempDir::new().expect("temp dir");
    let file = tmp.path().join("cache.json");
    std::fs::write(&file, "v1").expect("write file");

    let before = std::fs::metadata(&file)
        .expect("stat file before advance")
        .modified()
        .expect("mtime before advance");
    advance_mtime(&file, SECS).expect("advance mtime by 10s");
    let after = std::fs::metadata(&file)
        .expect("stat file after advance")
        .modified()
        .expect("mtime after advance");

    let floor = before
        .checked_add(Duration::from_secs(SECS))
        .and_then(|t| t.checked_sub(EPSILON))
        .expect("mtime arithmetic must not overflow");
    assert!(
        after >= floor,
        "推进后 mtime 必须 ≥ 原 mtime + {SECS}s − ε：before={before:?} after={after:?}"
    );
    assert!(after > before, "推进后 mtime 必须严格晚于原值");
}

/// 正常路径（零推进）：secs=0 时把 mtime 设为其当前值，读回应与推进前
/// 完全相等——原值本就取自文件系统（已对齐到存储粒度），回写同一值
/// 无粒度损失，mtime 不变。
#[test]
fn test_advance_mtime_zero_secs_keeps_mtime_unchanged() {
    let tmp = tempfile::TempDir::new().expect("temp dir");
    let file = tmp.path().join("cache.json");
    std::fs::write(&file, "v1").expect("write file");

    let before = std::fs::metadata(&file)
        .expect("stat file before advance")
        .modified()
        .expect("mtime before advance");
    advance_mtime(&file, 0).expect("advance mtime by 0s");
    let after = std::fs::metadata(&file)
        .expect("stat file after advance")
        .modified()
        .expect("mtime after advance");

    assert_eq!(after, before, "secs=0 时 mtime 必须保持不变");
}

/// 错误路径：路径不存在时打开文件失败，`advance_mtime` 以 `io::Result`
/// 原样透传返回 Err（不 panic），错误处理权留给调用者。
#[test]
fn test_advance_mtime_missing_path_returns_error() {
    let tmp = tempfile::TempDir::new().expect("temp dir");
    let missing = tmp.path().join("never-created.json");

    let result = advance_mtime(&missing, 10);
    assert!(result.is_err(), "路径不存在必须返回 Err 而非 panic");
}

/// TempDir 契约：debug 构建下路径落在 `std::env::temp_dir()` 之外时在
/// 任何 mtime 改写前快速失败（panic 前缀与 helper 名一致），与
/// `ensure_dir` / `remove_subdir` / `create_subdir` 的契约口径对齐。
#[test]
#[cfg(debug_assertions)]
#[should_panic(expected = "advance_mtime")]
fn test_advance_mtime_panics_for_path_outside_tempdir() {
    let outside = outside_tempdir_probe("closeclaw_advance_mtime_outside_probe");
    advance_mtime(&outside, 10).expect("advance_mtime: mtime advance outside temp dir must fail");
}

/// 边界：同一文件连续两次推进 +10s，第二次以第一次推进后的 mtime 为
/// 基准再 +10s，最终 mtime 相比初始值累计 ≥ +20s − ε（ε = 1s，容忍
/// 文件系统时间戳粒度截断，与单次推进测试的断言口径对齐）。
#[test]
fn test_advance_mtime_twice_accumulates_twenty_seconds() {
    const EPSILON: Duration = Duration::from_secs(1);

    let tmp = tempfile::TempDir::new().expect("temp dir");
    let file = tmp.path().join("cache.json");
    std::fs::write(&file, "v1").expect("write file");

    let before = std::fs::metadata(&file)
        .expect("stat file before advances")
        .modified()
        .expect("mtime before advances");
    advance_mtime(&file, 10).expect("first advance by 10s");
    advance_mtime(&file, 10).expect("second advance by 10s");
    let after = std::fs::metadata(&file)
        .expect("stat file after advances")
        .modified()
        .expect("mtime after advances");

    let gained = after
        .duration_since(before)
        .expect("mtime must stay monotone across advances");
    let floor = Duration::from_secs(20)
        .checked_sub(EPSILON)
        .expect("duration arithmetic must not overflow");
    assert!(
        gained >= floor,
        "两次 +10s 推进必须累计 ≥ +20s − ε：before={before:?} after={after:?} gained={gained:?}"
    );
}

/// 正常路径：log 多条后 `query_entries` 经 `AuditLogFilter` 命中正确子集
/// ——agent_id 单维命中全部该 agent 条目；agent_id + disposition 双维
/// 只保留同时匹配的条目（时间戳逐一核对，非仅数量断言）。
#[test]
fn test_mem_audit_logger_query_filters_matching_subset() {
    let logger = MemAuditLogger::new();
    log_approved(&logger, "a1", "2026-01-01T00:00:00Z");
    log_approved(&logger, "a2", "2026-01-02T00:00:00Z");
    logger.log(&sample_audit_entry(
        "a1",
        "2026-01-03T00:00:00Z",
        AuditDisposition::Rejected,
    ));
    log_approved(&logger, "a1", "2026-01-04T00:00:00Z");
    logger.log(&sample_audit_entry(
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
        log_approved(&logger, &format!("agent-{i}"), ts);
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
        log_approved(&logger, agent, ts);
    }

    let queried = logger.query_entries(&AuditLogFilter::default());
    let agents = agent_order(&queried);
    assert_eq!(
        agents,
        vec![
            "a-z00-03z",    // 03:00Z（原文 03:00+00:00）
            "a-plus8-02z",  // 02:00Z（10:00+08:00）
            "a-z-01z",      // 01:00Z（原文 01:00:00Z）
            "a-plus8-00z",  // 00:00Z（08:00+08:00）
            "a-z-prev-day", // 前一日 23:00Z
        ],
        "query 必须按解析后的真实时刻降序，而非 timestamp 字符串字典序"
    );
}

/// 边界（部分解析失败）：可解析条目按真实时刻降序在前，解析失败的
/// 条目无可比时刻、稳定地排在末尾（相互保持插入序）——默认 filter
/// 无时间窗，垃圾时间戳不影响 matches 判定，只决定排序位置。
#[test]
fn test_mem_audit_logger_query_unparseable_timestamps_sort_last_stable() {
    let logger = MemAuditLogger::new();
    let inserted = [
        ("g1", "not-a-time"),
        ("v1", "2026-01-01T00:00:00Z"),
        ("g2", "2026/01/02 03:04:05"),
        ("v2", "2026-01-03T00:00:00Z"),
        ("g3", "yesterday-ish"),
    ];
    for (agent, ts) in inserted {
        log_approved(&logger, agent, ts);
    }

    let queried = logger.query_entries(&AuditLogFilter::default());
    let agents = agent_order(&queried);
    assert_eq!(
        agents,
        vec!["v2", "v1", "g1", "g2", "g3"],
        "可解析条目按时刻降序在前，解析失败条目稳定排末尾（保持插入序）"
    );
}

/// 边界（全部解析失败）：所有 timestamp 都不可解析时没有可比时刻，
/// query 不得 panic 也不得重排——稳定排序让全部条目保持插入序。
#[test]
fn test_mem_audit_logger_query_all_unparseable_keeps_insertion_order() {
    let logger = MemAuditLogger::new();
    let inserted = [
        ("g1", "not-a-time"),
        ("g2", "2026/01/02 03:04:05"),
        ("g3", ""),
    ];
    for (agent, ts) in inserted {
        log_approved(&logger, agent, ts);
    }

    let queried = logger.query_entries(&AuditLogFilter::default());
    let agents = agent_order(&queried);
    assert_eq!(
        agents,
        vec!["g1", "g2", "g3"],
        "全部条目无可比时刻时必须保持插入序（稳定排序）"
    );
}

/// 同刻稳定性：解析到同一时刻的条目（含同刻不同形态 `00:00:00Z` vs
/// `08:00:00+08:00` vs `00:00:00+00:00`）在 query 结果中保持插入序，
/// 唯一更晚的条目排最前——`sort_by_key` 稳定排序承诺的行为（sort
/// 注释与 commit message 均已声明同刻保持插入序）。
#[test]
fn test_mem_audit_logger_query_same_instant_keeps_insertion_order() {
    let logger = MemAuditLogger::new();
    let inserted = [
        ("s1-z", "2026-01-02T00:00:00Z"),
        ("s2-plus8", "2026-01-02T08:00:00+08:00"), // 同刻不同形态
        ("s3-z00", "2026-01-02T00:00:00+00:00"),   // 同刻第三种形态
        ("s4-later", "2026-01-02T00:00:01Z"),      // 晚 1 秒，验证倒序
    ];
    for (agent, ts) in inserted {
        log_approved(&logger, agent, ts);
    }

    let queried = logger.query_entries(&AuditLogFilter::default());
    let agents = agent_order(&queried);
    assert_eq!(
        agents,
        vec!["s4-later", "s1-z", "s2-plus8", "s3-z00"],
        "更晚条目排最前，同刻条目（含同刻不同形态）保持插入序"
    );
}

/// 口径一致性（错误路径）：混合时区形态 + 垃圾时间戳的同批 entries，
/// 带 since/until 时间窗（界本身也是混合形态）的 query 结果必须与
/// `matches` 逐条判定完全一致（过滤子集相等，垃圾时间戳双双排除），
/// 且窗口内结果按解析时刻降序——证明过滤与排序同用「解析后的时刻」
/// 口径，二者不会漂移。
#[test]
fn test_mem_audit_logger_query_window_consistent_with_matches_ordering() {
    let logger = MemAuditLogger::new();
    let inserted = [
        ("w1-in-02z", "2026-01-02T10:00:00+08:00"), // 02:00Z
        ("w2-before", "2026-01-01T23:00:00Z"),
        ("w3-in-0130z", "2026-01-02T09:30:00+08:00"), // 01:30Z
        ("w4-garbage", "yesterday-ish"),
        ("w5-at-since", "2026-01-02T00:00:00+00:00"), // 00:00Z，恰在 since
        ("w6-after", "2026-01-02T16:00:00+08:00"),    // 08:00Z
        ("w7-in-01z", "2026-01-02T01:00:00Z"),
    ];
    for (agent, ts) in inserted {
        log_approved(&logger, agent, ts);
    }

    let window = AuditLogFilter {
        since: Some("2026-01-02T08:00:00+08:00".to_string()), // 00:00Z
        until: Some("2026-01-02T03:00:00Z".to_string()),
        ..Default::default()
    };
    let queried = logger.query_entries(&window);
    let agents = agent_order(&queried);
    assert_eq!(
        agents,
        vec!["w1-in-02z", "w3-in-0130z", "w7-in-01z", "w5-at-since"],
        "窗口内条目必须按解析时刻降序（含端点），垃圾时间戳被排除"
    );

    // 一致性：query 的过滤子集必须与「entries() 逐条 matches」完全相等
    //（同一批条目，仅序不同——序已由上一断言按时刻降序核对），
    // 且返回的每一条都单独通过 matches。
    let snapshot = logger.entries();
    let mut from_matches: Vec<&str> = snapshot
        .iter()
        .filter(|e| window.matches(e))
        .map(|e| e.agent_id.as_str())
        .collect();
    from_matches.sort_unstable();
    let mut queried_set = agent_order(&queried);
    queried_set.sort_unstable();
    assert_eq!(
        queried_set, from_matches,
        "query 过滤子集必须与 matches 逐条判定一致"
    );
    assert!(
        queried.iter().all(|e| window.matches(e)),
        "返回的每一条都必须通过 matches 判定"
    );
}

/// 快照语义：`entries()` 返回全量快照且与后续写入隔离——拍快照后
/// 继续 log，快照不变；修改快照 Vec 也不得穿透到 logger 内部状态。
#[test]
fn test_mem_audit_logger_entries_snapshot_isolated_from_later_writes() {
    let logger = MemAuditLogger::new();
    log_approved(&logger, "a1", "2026-01-01T00:00:00Z");
    logger.log(&sample_audit_entry(
        "a2",
        "2026-01-02T00:00:00Z",
        AuditDisposition::Rejected,
    ));

    let snapshot = logger.entries();
    assert_eq!(snapshot.len(), 2, "快照必须包含全部已写入条目");
    let agents = agent_order(&snapshot);
    assert_eq!(agents, vec!["a1", "a2"], "快照按插入序包含全部条目");

    log_approved(&logger, "a3", "2026-01-03T00:00:00Z");
    assert_eq!(snapshot.len(), 2, "后续写入不得改变已取得的快照");

    let mut mutable = logger.entries();
    mutable.clear();
    let fresh = logger.entries();
    assert_eq!(fresh.len(), 3, "修改快照 Vec 不得穿透到 logger 内部");
    let fresh_agents = agent_order(&fresh);
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
    log_approved(&logger, "a1", "2026-01-02T00:00:00Z");

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
                    logger.log(&sample_audit_entry(
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
