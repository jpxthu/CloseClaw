//! Multi-tool parallel dispatcher.
//!
//! [`ToolCallDispatcher`] routes a batch of tool calls to parallel, mutex-by-file,
//! or serial execution based on [`ToolFlags::is_concurrency_safe`] and per-file
//! mutex ownership.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use async_trait::async_trait;
use futures::future::join_all;

use crate::file_mutex::FileMutexMap;

// ---------------------------------------------------------------------------
// Input
// ---------------------------------------------------------------------------

/// A tool call waiting to be dispatched.
#[derive(Debug, Clone)]
pub struct PendingToolCall {
    /// Caller-assigned identifier (preserved in the result).
    ///
    /// Must be unique across all calls in a single `dispatch_all` invocation.
    pub id: String,
    /// Registered tool name (e.g. "Read", "Edit").
    pub tool_name: String,
    /// Target file path, if any.
    pub file_path: Option<PathBuf>,
    /// Whether the tool declares itself concurrency-safe.
    pub is_concurrency_safe: bool,
}

// ---------------------------------------------------------------------------
// Executor trait
// ---------------------------------------------------------------------------

/// Abstraction over actual tool execution.
///
/// The dispatcher never calls tools directly — callers inject an implementation
/// of this trait, which makes the dispatcher unit-testable without real I/O.
#[async_trait]
pub trait ToolExecutor: Send + Sync {
    /// Execute a single tool call and return its result.
    async fn execute(&self, call: &PendingToolCall) -> closeclaw_common::tool_trait::ToolResult;
}

// ---------------------------------------------------------------------------
// Dispatch groups
// ---------------------------------------------------------------------------

/// How a single tool call should be dispatched.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DispatchGroup {
    /// Can run in parallel with any other call.
    Parallel,
    /// Must be serialized per file path (Write/Edit on the same file).
    MutexByFile,
    /// Must run one-by-one (no concurrency support, no file path).
    Serial,
}

// ===========================================================================
// Classification logic
// ===========================================================================

/// Routes a batch of tool calls to the appropriate execution strategy.
pub struct ToolCallDispatcher {
    file_mutex_map: Arc<FileMutexMap>,
    is_parallel_enabled: bool,
}

impl ToolCallDispatcher {
    /// Create a new dispatcher with the given mutex map and parallel switch.
    pub fn new(file_mutex_map: Arc<FileMutexMap>, is_parallel_enabled: bool) -> Self {
        Self {
            file_mutex_map,
            is_parallel_enabled,
        }
    }

    /// Whether all calls should fall back to serial execution.
    pub fn should_fallback_to_serial(&self, provider_supports_parallel: bool) -> bool {
        !self.is_parallel_enabled || !provider_supports_parallel
    }

    /// Classify a tool call into its dispatch group.
    pub fn classify(&self, call: &PendingToolCall) -> DispatchGroup {
        if !self.is_parallel_enabled {
            return DispatchGroup::Serial;
        }
        if call.is_concurrency_safe {
            return DispatchGroup::Parallel;
        }
        if call.file_path.is_some() {
            return DispatchGroup::MutexByFile;
        }
        DispatchGroup::Serial
    }

    /// Classify all calls, returning `(index, group)` pairs.
    fn classify_calls(&self, calls: &[PendingToolCall]) -> Vec<(usize, DispatchGroup)> {
        calls
            .iter()
            .enumerate()
            .map(|(i, c)| (i, self.classify(c)))
            .collect()
    }

    /// Extract calls belonging to a specific dispatch group.
    fn filter_group<'a>(
        classified: &[(usize, DispatchGroup)],
        calls: &'a [PendingToolCall],
        group: DispatchGroup,
    ) -> Vec<&'a PendingToolCall> {
        classified
            .iter()
            .filter(|(_, g)| *g == group)
            .map(|(i, _)| &calls[*i])
            .collect()
    }

    /// Split parallel calls into two buckets:
    /// - **ordered**: calls whose file also appears in the mutex set (Read
    ///   that must precede a matching Edit).
    /// - **remaining**: all other parallel calls.
    fn reorder_read_edit<'a>(
        parallel: Vec<&'a PendingToolCall>,
        mutex_files: &HashSet<&Path>,
    ) -> (Vec<&'a PendingToolCall>, Vec<&'a PendingToolCall>) {
        parallel.into_iter().partition(|c| {
            c.file_path
                .as_deref()
                .is_some_and(|p| mutex_files.contains(p))
        })
    }
}

// ===========================================================================
// Execution logic
// ===========================================================================

impl ToolCallDispatcher {
    /// Execute a group of calls concurrently and return `(id, result)` pairs.
    async fn execute_parallel(
        calls: &[&PendingToolCall],
        executor: &dyn ToolExecutor,
    ) -> Vec<(String, closeclaw_common::tool_trait::ToolResult)> {
        let futures: Vec<_> = calls.iter().map(|call| executor.execute(call)).collect();
        let results = join_all(futures).await;
        calls
            .iter()
            .zip(results)
            .map(|(call, res)| (call.id.clone(), res))
            .collect()
    }

    /// Execute mutex-by-file calls: serialize per file, parallel across files.
    async fn execute_mutex_by_file(
        calls: &[&PendingToolCall],
        executor: &dyn ToolExecutor,
        mutex_map: &Arc<FileMutexMap>,
    ) -> Vec<(String, closeclaw_common::tool_trait::ToolResult)> {
        let mut by_file: std::collections::HashMap<PathBuf, Vec<&PendingToolCall>> =
            std::collections::HashMap::new();
        for call in calls {
            let path = call.file_path.clone().unwrap();
            by_file.entry(path).or_default().push(call);
        }

        let file_futures: Vec<_> = by_file
            .into_iter()
            .map(|(path, file_calls)| {
                let mutex_map = Arc::clone(mutex_map);
                async move {
                    let mut results = Vec::new();
                    for call in file_calls {
                        let guard = mutex_map.acquire(&path).await;
                        let res = executor.execute(call).await;
                        drop(guard);
                        mutex_map.cleanup(&path);
                        results.push((call.id.clone(), res));
                    }
                    results
                }
            })
            .collect();
        let file_results = join_all(file_futures).await;
        file_results.into_iter().flatten().collect()
    }

    /// Execute calls one-by-one in order.
    async fn execute_serial(
        calls: &[&PendingToolCall],
        executor: &dyn ToolExecutor,
    ) -> Vec<(String, closeclaw_common::tool_trait::ToolResult)> {
        let mut results = Vec::with_capacity(calls.len());
        for call in calls {
            let res = executor.execute(call).await;
            results.push((call.id.clone(), res));
        }
        results
    }
}

// ===========================================================================
// Orchestration logic
// ===========================================================================

impl ToolCallDispatcher {
    /// Dispatch all tool calls and return results in input order.
    ///
    /// Calls are classified into three groups:
    /// - **Parallel**: executed concurrently via [`join_all`].
    /// - **MutexByFile**: grouped by file path; same-file calls are
    ///   serialized, different-file calls run in parallel.
    /// - **Serial**: executed one-by-one in order.
    ///
    /// Read calls targeting a file that also has MutexByFile calls are
    /// reordered to execute first (Edit depends on Read's mtime).
    pub async fn dispatch_all(
        &self,
        calls: Vec<PendingToolCall>,
        executor: &dyn ToolExecutor,
    ) -> Vec<closeclaw_common::tool_trait::ToolResult> {
        if calls.is_empty() {
            return Vec::new();
        }

        let classified = self.classify_calls(&calls);
        let parallel = Self::filter_group(&classified, &calls, DispatchGroup::Parallel);
        let mutex = Self::filter_group(&classified, &calls, DispatchGroup::MutexByFile);
        let serial = Self::filter_group(&classified, &calls, DispatchGroup::Serial);

        let mutex_files: HashSet<&Path> = mutex
            .iter()
            .filter_map(|c| c.file_path.as_deref())
            .collect();
        let (ordered, remaining) = Self::reorder_read_edit(parallel, &mutex_files);

        let id_to_index: std::collections::HashMap<&str, usize> = calls
            .iter()
            .enumerate()
            .map(|(i, c)| (c.id.as_str(), i))
            .collect();
        let mut results: Vec<Option<closeclaw_common::tool_trait::ToolResult>> =
            vec![None; calls.len()];

        let mut fill = |v: Vec<(String, _)>| {
            Self::fill_results(&mut results, &id_to_index, v);
        };

        if !ordered.is_empty() {
            fill(Self::execute_parallel(&ordered, executor).await);
        }
        if !remaining.is_empty() {
            fill(Self::execute_parallel(&remaining, executor).await);
        }
        if !mutex.is_empty() {
            fill(Self::execute_mutex_by_file(&mutex, executor, &self.file_mutex_map).await);
        }
        fill(Self::execute_serial(&serial, executor).await);

        results
            .into_iter()
            .map(|r| r.expect("every call should have a result"))
            .collect()
    }

    /// Fill results vector from `(id, result)` pairs using a pre-built index map.
    fn fill_results(
        results: &mut [Option<closeclaw_common::tool_trait::ToolResult>],
        id_to_index: &std::collections::HashMap<&str, usize>,
        pairs: Vec<(String, closeclaw_common::tool_trait::ToolResult)>,
    ) {
        for (id, res) in pairs {
            results[*id_to_index.get(id.as_str()).unwrap()] = Some(res);
        }
    }
}

#[cfg(test)]
#[path = "dispatcher_tests.rs"]
mod tests;
