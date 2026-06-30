use std::path::Path;

use crate::client::{admin_socket_path, AdminClient};
use crate::protocol::AdminRequest;

const ADMIN_TIMEOUT_MS: u64 = 5000;

#[test]
fn test_admin_client_new() {
    let client = AdminClient::new("/tmp/test.sock");
    assert_eq!(client.path, "/tmp/test.sock");
    assert_eq!(client.timeout_ms, ADMIN_TIMEOUT_MS);
}

#[test]
fn test_admin_client_with_timeout() {
    let client = AdminClient::with_timeout("/tmp/test.sock", 1000);
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
    let client = AdminClient::new("/tmp/nonexistent-admin-test.sock");
    let result = client.call(&AdminRequest::Ping).await;
    assert!(result.is_err());
}

#[tokio::test]
async fn test_client_ping_to_nonexistent_socket() {
    let client = AdminClient::new("/tmp/nonexistent-admin-test.sock");
    assert!(!client.ping().await);
}
