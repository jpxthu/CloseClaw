//! Tests for the composition-root outbound raw-log writer seam.

use std::collections::HashMap;
use std::path::Path;

use closeclaw_gateway::outbound_raw_log::OutboundRawLogSnapshot;
use closeclaw_gateway::GatewayConfig;

fn config(raw_log_dir: Option<std::path::PathBuf>) -> GatewayConfig {
    GatewayConfig {
        name: "test-outbound-raw-log".to_string(),
        raw_log_dir,
        ..Default::default()
    }
}

/// The composition root only offers a writer when `raw_log_dir` is
/// configured — the same gate the Gateway applied before the seam existed.
#[test]
fn test_build_outbound_raw_log_writer_gates_on_raw_log_dir() {
    assert!(
        crate::outbound_raw_log::build_outbound_raw_log_writer(&config(None)).is_none(),
        "no raw_log_dir → nothing to inject"
    );

    let tmp = tempfile::tempdir().expect("tempdir");
    let configured = config(Some(tmp.path().to_path_buf()));
    assert!(
        crate::outbound_raw_log::build_outbound_raw_log_writer(&configured).is_some(),
        "raw_log_dir configured → writer must be injected"
    );
}

/// The injected writer produces the very same snapshot file the direct
/// processor-chain call used to: `{channel}_outbound_{ts}_{message_id}.json`
/// containing `content`, `content_blocks_summary` and `metadata`.
#[tokio::test]
async fn test_injected_writer_writes_outbound_snapshot_json() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let writer = crate::outbound_raw_log::build_outbound_raw_log_writer(&config(Some(
        tmp.path().to_path_buf(),
    )))
    .expect("raw_log_dir configured → writer injected");

    let mut metadata = HashMap::new();
    metadata.insert("channel".to_string(), "mock".to_string());
    writer
        .write(OutboundRawLogSnapshot {
            content: "hello world".to_string(),
            content_blocks: vec![closeclaw_common::ContentBlock::Text(
                "hello world".to_string(),
            )],
            metadata,
        })
        .await
        .expect("snapshot write must succeed");

    let mut entries: Vec<std::path::PathBuf> = std::fs::read_dir(tmp.path())
        .expect("read_dir")
        .map(|e| e.expect("dir entry").path())
        .collect();
    entries.sort();
    assert_eq!(entries.len(), 1, "exactly one snapshot file expected");
    let path = &entries[0];
    let name = file_name(path);
    assert!(
        name.starts_with("mock_outbound_"),
        "filename scheme must stay `{{channel}}_outbound_*`, got {name}"
    );

    let raw = std::fs::read_to_string(path).expect("snapshot must be readable");
    let value: serde_json::Value = serde_json::from_str(&raw).expect("snapshot must be JSON");
    assert_eq!(value["content"], "hello world");
    assert_eq!(value["metadata"]["channel"], "mock");
    assert_eq!(
        value["content_blocks_summary"]
            .as_array()
            .map(Vec::len)
            .unwrap_or_default(),
        1,
        "content_blocks_summary must carry one entry per block"
    );
}

fn file_name(path: &Path) -> String {
    path.file_name()
        .expect("file name")
        .to_string_lossy()
        .into_owned()
}
