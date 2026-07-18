//! IPC channel for communication with the sandboxed Permission Engine.
//!
//! Uses length-prefixed JSON frames over a Unix domain socket:
//!
//! ```text
//! [4-byte big-endian length (u32)][JSON frame bytes]
//! ```

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use tokio::io::{AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::net::unix::OwnedWriteHalf;
use tokio::net::{UnixListener, UnixStream};
use tokio::time::timeout;

use crate::{PermissionEngine, PermissionRequest, PermissionResponse, RuleSet};

// ---------------------------------------------------------------------------
// IPC Constants
// ---------------------------------------------------------------------------

/// Maximum time to wait for a response from the engine over IPC.
pub const IPC_TIMEOUT_MS: u64 = 3000;

// ---------------------------------------------------------------------------
// IPC Messages
// ---------------------------------------------------------------------------

/// Request sent from the host process to the sandboxed engine.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum SandboxRequest {
    /// Ask the engine to evaluate a permission request.
    Evaluate {
        request: PermissionRequest,
        #[serde(default)]
        extra_deny_subjects: Option<Vec<crate::engine::engine_types::Subject>>,
    },
    /// Ask the engine to reload its ruleset.
    ReloadRules { rules: RuleSet },
    /// Ping — returns Pong.
    Ping,
}

/// Response sent from the sandboxed engine back to the host.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum SandboxResponse {
    /// Permission evaluation result.
    PermissionResponse(PermissionResponse),
    /// Acknowledge a rules reload.
    RulesReloaded,
    /// Pong acknowledgement.
    Pong,
    /// Engine-side error.
    Error { message: String },
}

// ---------------------------------------------------------------------------
// IPC Channel
// ---------------------------------------------------------------------------

/// Bidirectional IPC channel over a Unix domain socket.
///
/// The protocol uses length-prefixed JSON frames:
///
/// ```text
/// [4-byte big-endian length (u32)][JSON frame bytes]
/// ```
pub struct IpcChannel {
    path: PathBuf,
}

impl IpcChannel {
    /// Create a new IPC channel at the given socket path.
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self { path: path.into() }
    }

    /// Remove the socket file if it already exists (idempotent).
    pub fn clean_up(&self) {
        let _ = std::fs::remove_file(&self.path);
    }

    /// Connect to the engine and send a request, returning the response.
    pub async fn call(&self, request: &SandboxRequest) -> std::io::Result<SandboxResponse> {
        let stream = timeout(
            Duration::from_millis(IPC_TIMEOUT_MS),
            UnixStream::connect(&self.path),
        )
        .await
        .map_err(|_| std::io::Error::new(std::io::ErrorKind::TimedOut, "IPC connect timeout"))??;

        let (reader, mut writer): (_, OwnedWriteHalf) = stream.into_split();

        // Send request: [4-byte len][json]
        let json = serde_json::to_vec(request)?;
        let len = (json.len() as u32).to_be_bytes();
        writer.write_all(&len).await?;
        writer.write_all(&json).await?;
        writer.flush().await?;

        // Read response header
        let mut hdr = [0u8; 4];
        let mut reader = BufReader::new(reader);
        reader.read_exact(&mut hdr).await?;
        let body_len = u32::from_be_bytes(hdr) as usize;

        // Read body
        let mut body = vec![0u8; body_len];
        reader.read_exact(&mut body).await?;

        serde_json::from_slice(&body)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))
    }

    /// Start listening for connections and dispatch them to the engine.
    ///
    /// Blocks forever, processing each connection in a spawned task.
    pub async fn serve(
        self,
        engine: Arc<tokio::sync::RwLock<PermissionEngine>>,
    ) -> std::io::Result<()> {
        self.clean_up();

        let listener = UnixListener::bind(&self.path)?;

        tracing::info!("engine IPC server listening on {}", self.path.display());

        loop {
            match listener.accept().await {
                Ok((stream, _)) => {
                    let engine = engine.clone();
                    tokio::spawn(async move {
                        if let Err(e) = handle_connection(stream, engine).await {
                            tracing::error!("IPC connection error: {}", e);
                        }
                    });
                }
                Err(e) => {
                    tracing::error!("UnixListener accept error: {}", e);
                }
            }
        }
    }
}

/// Handle a single IPC connection from the host.
async fn handle_connection(
    stream: UnixStream,
    engine: Arc<tokio::sync::RwLock<PermissionEngine>>,
) -> std::io::Result<()> {
    let (reader, writer): (_, OwnedWriteHalf) = stream.into_split();
    let mut reader = BufReader::new(reader);
    let mut writer = writer;

    loop {
        // Read 4-byte length header
        let mut hdr = [0u8; 4];
        match reader.read_exact(&mut hdr).await {
            Ok(_) => {}
            Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof => break,
            Err(e) => return Err(e),
        }
        let body_len = u32::from_be_bytes(hdr) as usize;

        // Read body
        let mut body = vec![0u8; body_len];
        reader.read_exact(&mut body).await?;

        // Deserialize request
        let request: SandboxRequest = match serde_json::from_slice(&body) {
            Ok(r) => r,
            Err(e) => {
                let resp = SandboxResponse::Error {
                    message: format!("invalid request: {}", e),
                };
                send_response(&mut writer, &resp).await?;
                continue;
            }
        };

        // Process request
        let response = match &request {
            SandboxRequest::Evaluate {
                request,
                extra_deny_subjects,
            } => SandboxResponse::PermissionResponse(
                engine
                    .read()
                    .await
                    .evaluate(request.clone(), extra_deny_subjects.clone()),
            ),
            SandboxRequest::ReloadRules { rules } => {
                engine.write().await.reload_rules(rules.clone());
                SandboxResponse::RulesReloaded
            }
            SandboxRequest::Ping => SandboxResponse::Pong,
        };

        send_response(&mut writer, &response).await?;
    }

    Ok(())
}

async fn send_response(
    writer: &mut OwnedWriteHalf,
    response: &SandboxResponse,
) -> std::io::Result<()> {
    let json = serde_json::to_vec(response)?;
    let len = (json.len() as u32).to_be_bytes();
    writer.write_all(&len).await?;
    writer.write_all(&json).await?;
    writer.flush().await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_sandbox_request_ping_serialization() {
        let req = SandboxRequest::Ping;
        let json = serde_json::to_vec(&req).unwrap();
        let deserialized: SandboxRequest = serde_json::from_slice(&json).unwrap();
        // Compare JSON string representation since contained types may not implement PartialEq
        assert_eq!(
            serde_json::to_string(&req).unwrap(),
            serde_json::to_string(&deserialized).unwrap()
        );
    }

    #[test]
    fn test_sandbox_response_pong_serialization() {
        let resp = SandboxResponse::Pong;
        let json = serde_json::to_vec(&resp).unwrap();
        let deserialized: SandboxResponse = serde_json::from_slice(&json).unwrap();
        assert_eq!(
            serde_json::to_string(&resp).unwrap(),
            serde_json::to_string(&deserialized).unwrap()
        );
    }

    #[test]
    fn test_sandbox_response_error_serialization() {
        let resp = SandboxResponse::Error {
            message: "test error message".to_string(),
        };
        let json = serde_json::to_vec(&resp).unwrap();
        let deserialized: SandboxResponse = serde_json::from_slice(&json).unwrap();
        assert_eq!(
            serde_json::to_string(&resp).unwrap(),
            serde_json::to_string(&deserialized).unwrap()
        );
    }

    #[test]
    fn test_sandbox_request_evaluate_serialization() {
        let req = SandboxRequest::Evaluate {
            request: crate::PermissionRequest::Bare(crate::PermissionRequestBody::FileOp {
                agent: "test-agent".to_string(),
                path: "/tmp/test.txt".to_string(),
                op: "read".to_string(),
            }),
            extra_deny_subjects: None,
        };
        let json = serde_json::to_vec(&req).unwrap();
        let deserialized: SandboxRequest = serde_json::from_slice(&json).unwrap();
        assert_eq!(
            serde_json::to_string(&req).unwrap(),
            serde_json::to_string(&deserialized).unwrap()
        );
    }

    #[test]
    fn test_sandbox_request_reload_rules_serialization() {
        let rules = crate::engine::RuleSet {
            rules: vec![],
            defaults: Default::default(),
            user_defaults: crate::engine::Defaults::user_defaults(),
            template_includes: vec![],
            agent_creators: Default::default(),
            rule_version: String::new(),
        };
        let req = SandboxRequest::ReloadRules { rules };
        let json = serde_json::to_vec(&req).unwrap();
        let deserialized: SandboxRequest = serde_json::from_slice(&json).unwrap();
        assert_eq!(
            serde_json::to_string(&req).unwrap(),
            serde_json::to_string(&deserialized).unwrap()
        );
    }

    #[test]
    fn test_sandbox_response_permission_allowed_serialization() {
        let resp = SandboxResponse::PermissionResponse(crate::PermissionResponse::Allowed {
            token: "token123".to_string(),
            context_modifier: None,
        });
        let json = serde_json::to_vec(&resp).unwrap();
        let deserialized: SandboxResponse = serde_json::from_slice(&json).unwrap();
        assert_eq!(
            serde_json::to_string(&resp).unwrap(),
            serde_json::to_string(&deserialized).unwrap()
        );
    }

    #[test]
    fn test_sandbox_response_rules_reloaded_serialization() {
        let resp = SandboxResponse::RulesReloaded;
        let json = serde_json::to_vec(&resp).unwrap();
        let deserialized: SandboxResponse = serde_json::from_slice(&json).unwrap();
        assert_eq!(
            serde_json::to_string(&resp).unwrap(),
            serde_json::to_string(&deserialized).unwrap()
        );
    }

    #[test]
    fn test_ipc_channel_new() {
        let channel = IpcChannel::new("/tmp/test-socket.sock");
        assert_eq!(
            channel.path,
            std::path::PathBuf::from("/tmp/test-socket.sock")
        );
    }

    #[test]
    fn test_ipc_channel_clean_up_nonexistent_path() {
        // clean_up on a non-existent path should not panic (idempotent)
        let channel = IpcChannel::new("/tmp/nonexistent-socket-12345.sock");
        channel.clean_up(); // should not panic
                            // Running it twice is also fine (idempotent)
        channel.clean_up();
    }

    // -----------------------------------------------------------------
    // ReloadRules IPC integration tests
    //
    // These tests verify that ReloadRules actually writes rules into the
    // engine (the fix from Step 1.1). They spin up a lightweight IPC
    // server in-process using IpcChannel::serve + a shared
    // PermissionEngine, then exercise the full IPC round-trip.
    // -----------------------------------------------------------------

    use crate::engine::{Action, Effect, MatchType, Rule, Subject};
    use std::sync::Arc;
    use tokio::sync::RwLock;
    use tokio::time::timeout;

    /// Helper: create a temp socket path using TempDir (safe, auto-cleaned).
    fn test_socket_path() -> (std::path::PathBuf, tempfile::TempDir) {
        let tmpdir = tempfile::TempDir::new().expect("tempdir");
        let path = tmpdir
            .path()
            .join(format!("ipc-test-{}.sock", std::process::id()));
        (path, tmpdir)
    }

    /// Start the IPC server on a temp socket, returning the IpcChannel for
    /// the caller to use.
    async fn start_ipc_server(
        engine: Arc<RwLock<crate::PermissionEngine>>,
        path: &std::path::PathBuf,
    ) {
        let server_channel = IpcChannel::new(path.clone());
        let engine_clone = engine.clone();
        tokio::spawn(async move {
            let _ = server_channel.serve(engine_clone).await;
        });
        // Give the server a moment to bind.
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }

    /// Build a FileOp permission request for a given agent.
    fn file_read_request(agent: &str) -> crate::PermissionRequest {
        crate::PermissionRequest::Bare(crate::PermissionRequestBody::FileOp {
            agent: agent.to_string(),
            path: "/tmp/test.txt".to_string(),
            op: "read".to_string(),
        })
    }

    /// Create an allow-all RuleSet: one rule with Subject::AgentOnly +
    /// MatchType::Glob that allows every action.
    fn allow_all_rules() -> crate::RuleSet {
        crate::RuleSet {
            rules: vec![Rule {
                name: "allow-all".to_string(),
                subject: Subject::AgentOnly {
                    agent: "*".to_string(),
                    match_type: MatchType::Glob,
                },
                effect: Effect::Allow,
                actions: vec![Action::All],
                template: None,
                priority: 0,
            }],
            defaults: crate::engine::Defaults {
                file_read: Effect::Allow,
                file_write: Effect::Allow,
                command: Effect::Allow,
                network: Effect::Allow,
                inter_agent: Effect::Allow,
                config: Effect::Allow,
                tool_call: Effect::Allow,
                message: Effect::Allow,
            },
            user_defaults: crate::engine::Defaults::user_defaults(),
            template_includes: vec![],
            agent_creators: Default::default(),
            rule_version: String::new(),
        }
    }

    /// Create a deny-all RuleSet: one rule that denies everything.
    fn deny_all_rules() -> crate::RuleSet {
        crate::RuleSet {
            rules: vec![Rule {
                name: "deny-all".to_string(),
                subject: Subject::AgentOnly {
                    agent: "*".to_string(),
                    match_type: MatchType::Glob,
                },
                effect: Effect::Deny,
                actions: vec![Action::All],
                template: None,
                priority: 0,
            }],
            defaults: crate::engine::Defaults::default(),
            user_defaults: crate::engine::Defaults::user_defaults(),
            template_includes: vec![],
            agent_creators: Default::default(),
            rule_version: String::new(),
        }
    }

    /// Create an empty RuleSet (no rules, default deny).
    fn empty_rules() -> crate::RuleSet {
        crate::RuleSet {
            rules: vec![],
            defaults: crate::engine::Defaults::default(),
            user_defaults: crate::engine::Defaults::user_defaults(),
            template_includes: vec![],
            agent_creators: Default::default(),
            rule_version: String::new(),
        }
    }

    // -- Test 1: ReloadRules with allow-all → Evaluate returns Allowed --

    #[tokio::test]
    async fn test_ipc_reload_rules_allow_all_then_evaluate_allowed() {
        let (socket_path, _tmpdir) = test_socket_path();
        let _ = std::fs::remove_file(&socket_path);

        // Start with empty (deny-all default) engine.
        let engine = Arc::new(RwLock::new(
            crate::PermissionEngine::new_with_default_data_root(empty_rules()),
        ));
        start_ipc_server(engine.clone(), &socket_path).await;

        let client = IpcChannel::new(&socket_path);

        // 1. Reload with allow-all rules.
        let resp = timeout(
            std::time::Duration::from_secs(3),
            client.call(&SandboxRequest::ReloadRules {
                rules: allow_all_rules(),
            }),
        )
        .await
        .expect("timeout")
        .expect("IPC error");
        assert!(matches!(resp, SandboxResponse::RulesReloaded));

        // 2. Evaluate — should be Allowed because rules are now injected.
        let resp = timeout(
            std::time::Duration::from_secs(3),
            client.call(&SandboxRequest::Evaluate {
                request: file_read_request("test-agent"),
                extra_deny_subjects: None,
            }),
        )
        .await
        .expect("timeout")
        .expect("IPC error");
        match resp {
            SandboxResponse::PermissionResponse(crate::PermissionResponse::Allowed { .. }) => {}
            other => panic!(
                "expected Allowed after ReloadRules(allow-all), got {:?}",
                other
            ),
        }
    }

    // -- Test 2: ReloadRules with deny-all → Evaluate returns Denied --

    #[tokio::test]
    async fn test_ipc_reload_rules_deny_all_then_evaluate_denied() {
        let (socket_path, _tmpdir) = test_socket_path();
        let _ = std::fs::remove_file(&socket_path);

        // Start with allow-all rules.
        let engine = Arc::new(RwLock::new(
            crate::PermissionEngine::new_with_default_data_root(allow_all_rules()),
        ));
        start_ipc_server(engine.clone(), &socket_path).await;

        let client = IpcChannel::new(&socket_path);

        // 1. Verify initial state is Allowed.
        let resp = timeout(
            std::time::Duration::from_secs(3),
            client.call(&SandboxRequest::Evaluate {
                request: file_read_request("test-agent"),
                extra_deny_subjects: None,
            }),
        )
        .await
        .expect("timeout")
        .expect("IPC error");
        assert!(
            matches!(
                resp,
                SandboxResponse::PermissionResponse(crate::PermissionResponse::Allowed { .. })
            ),
            "initial state should be Allowed"
        );

        // 2. Reload with deny-all rules.
        let resp = timeout(
            std::time::Duration::from_secs(3),
            client.call(&SandboxRequest::ReloadRules {
                rules: deny_all_rules(),
            }),
        )
        .await
        .expect("timeout")
        .expect("IPC error");
        assert!(matches!(resp, SandboxResponse::RulesReloaded));

        // 3. Evaluate — should be Denied because deny-all rules are injected.
        let resp = timeout(
            std::time::Duration::from_secs(3),
            client.call(&SandboxRequest::Evaluate {
                request: file_read_request("test-agent"),
                extra_deny_subjects: None,
            }),
        )
        .await
        .expect("timeout")
        .expect("IPC error");
        match resp {
            SandboxResponse::PermissionResponse(crate::PermissionResponse::Denied {
                reason,
                ..
            }) => {
                assert!(
                    reason.contains("denied"),
                    "denied reason should mention denied: {}",
                    reason
                );
            }
            other => panic!(
                "expected Denied after ReloadRules(deny-all), got {:?}",
                other
            ),
        }
    }

    // -- Test 3: ReloadRules with empty RuleSet → engine back to default Deny --

    #[tokio::test]
    async fn test_ipc_reload_rules_empty_setback_to_default_deny() {
        let (socket_path, _tmpdir) = test_socket_path();
        let _ = std::fs::remove_file(&socket_path);

        // Start with allow-all rules.
        let engine = Arc::new(RwLock::new(
            crate::PermissionEngine::new_with_default_data_root(allow_all_rules()),
        ));
        start_ipc_server(engine.clone(), &socket_path).await;

        let client = IpcChannel::new(&socket_path);

        // 1. Verify initial state is Allowed.
        let resp = timeout(
            std::time::Duration::from_secs(3),
            client.call(&SandboxRequest::Evaluate {
                request: file_read_request("test-agent"),
                extra_deny_subjects: None,
            }),
        )
        .await
        .expect("timeout")
        .expect("IPC error");
        assert!(
            matches!(
                resp,
                SandboxResponse::PermissionResponse(crate::PermissionResponse::Allowed { .. })
            ),
            "initial state should be Allowed"
        );

        // 2. Reload with empty rules (back to defaults = deny).
        let resp = timeout(
            std::time::Duration::from_secs(3),
            client.call(&SandboxRequest::ReloadRules {
                rules: empty_rules(),
            }),
        )
        .await
        .expect("timeout")
        .expect("IPC error");
        assert!(matches!(resp, SandboxResponse::RulesReloaded));

        // 3. Evaluate — should be Denied (default deny, no matching rule).
        let resp = timeout(
            std::time::Duration::from_secs(3),
            client.call(&SandboxRequest::Evaluate {
                request: file_read_request("test-agent"),
                extra_deny_subjects: None,
            }),
        )
        .await
        .expect("timeout")
        .expect("IPC error");
        match resp {
            SandboxResponse::PermissionResponse(crate::PermissionResponse::Denied {
                reason,
                ..
            }) => {
                assert!(
                    reason.contains("no matching rule") || reason.contains("denied"),
                    "denied reason should indicate no matching rule: {}",
                    reason
                );
            }
            other => panic!("expected Denied after ReloadRules(empty), got {:?}", other),
        }
    }
}
