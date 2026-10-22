use std::path::Path;

use crate::admin::rpc::client::{admin_socket_path, AdminClient};
use crate::admin::rpc::protocol::AdminRequest;

const ADMIN_TIMEOUT_MS: u64 = 5000;

#[test]
fn test_admin_client_new() {
    let sock = std::env::temp_dir()
        .join("test.sock")
        .to_string_lossy()
        .into_owned();
    let client = AdminClient::new(sock.clone());
    assert_eq!(client.path, sock);
    assert_eq!(client.timeout_ms, ADMIN_TIMEOUT_MS);
}

#[test]
fn test_admin_client_with_timeout() {
    let sock = std::env::temp_dir().join("test-timeout.sock");
    let client = AdminClient::with_timeout(sock.to_string_lossy().into_owned(), 1000);
    assert_eq!(client.timeout_ms, 1000);
}

#[test]
fn test_admin_socket_path() {
    let path = admin_socket_path(Path::new("/home/user/.closeclaw"));
    assert_eq!(
        path,
        std::path::PathBuf::from("/home/user/.closeclaw/admin.sock")
    );
}

#[tokio::test]
async fn test_client_connect_to_nonexistent_socket() {
    let sock = std::env::temp_dir()
        .join("nonexistent-admin-test.sock")
        .to_string_lossy()
        .into_owned();
    let client = AdminClient::new(sock);
    let result = client.call(&AdminRequest::Ping).await;
    assert!(result.is_err());
}

#[tokio::test]
async fn test_client_ping_to_nonexistent_socket() {
    let sock = std::env::temp_dir()
        .join("nonexistent-admin-test.sock")
        .to_string_lossy()
        .into_owned();
    let client = AdminClient::new(sock);
    assert!(!client.ping().await);
}
