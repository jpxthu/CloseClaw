//! Unit tests for CommandSandbox routing and script detection.

use super::CommandSandbox;
use tempfile::TempDir;

// ---------------------------------------------------------------------------
// Script detection
// ---------------------------------------------------------------------------

#[test]
fn test_script_sh_detected() {
    assert!(CommandSandbox::should_sandbox("script.sh", true));
}

#[test]
fn test_script_py_detected() {
    assert!(CommandSandbox::should_sandbox("run.py", true));
}

#[test]
fn test_script_pl_detected() {
    assert!(CommandSandbox::should_sandbox("deploy.pl", true));
}

#[test]
fn test_script_rb_detected() {
    assert!(CommandSandbox::should_sandbox("build.rb", true));
}

#[test]
fn test_script_js_detected() {
    assert!(CommandSandbox::should_sandbox("test.js", true));
}

#[test]
fn test_script_with_args_detected() {
    assert!(CommandSandbox::should_sandbox("script.sh --verbose", true));
}

#[test]
fn test_script_with_full_path_detected() {
    assert!(CommandSandbox::should_sandbox(
        "/usr/local/bin/run.py",
        true
    ));
}

#[test]
fn test_non_script_not_sandboxed_when_permitted() {
    assert!(!CommandSandbox::should_sandbox("echo hello", true));
}

#[test]
fn test_non_script_sandboxed_when_not_permitted() {
    assert!(CommandSandbox::should_sandbox("rm -rf /", false));
}

// ---------------------------------------------------------------------------
// Routing
// ---------------------------------------------------------------------------

#[test]
fn test_permitted_command_runs_outside() {
    let tmp = TempDir::new().unwrap();
    let result = CommandSandbox::route_command("echo ok", true, tmp.path().to_str().unwrap());
    assert!(result.is_ok());
    assert!(result.unwrap().trim() == "ok");
}

#[test]
fn test_unpermitted_command_runs_in_sandbox() {
    let tmp = TempDir::new().unwrap();
    let result =
        CommandSandbox::route_command("echo sandboxed", false, tmp.path().to_str().unwrap());
    assert!(result.is_ok());
    assert!(result.unwrap().trim() == "sandboxed");
}

#[test]
fn test_script_always_runs_in_sandbox() {
    let tmp = TempDir::new().unwrap();
    // Write a script to execute
    let script = tmp.path().join("test.sh");
    std::fs::write(&script, "#!/bin/sh\necho script_output").unwrap();
    // Make the script executable
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
    }
    let result =
        CommandSandbox::route_command(script.to_str().unwrap(), true, tmp.path().to_str().unwrap());
    assert!(result.is_ok());
    assert_eq!(result.unwrap().trim(), "script_output");
}

// ---------------------------------------------------------------------------
// Edge cases
// ---------------------------------------------------------------------------

#[test]
fn test_empty_command_is_not_script() {
    assert!(!CommandSandbox::should_sandbox("", true));
}

#[test]
fn test_whitespace_only_is_not_script() {
    assert!(!CommandSandbox::should_sandbox("   ", true));
}

#[test]
fn test_command_with_semicolons() {
    let tmp = TempDir::new().unwrap();
    let result =
        CommandSandbox::route_command("echo a; echo b", true, tmp.path().to_str().unwrap());
    assert!(result.is_ok());
    let output = result.unwrap();
    assert!(output.contains("a"));
    assert!(output.contains("b"));
}

#[test]
fn test_command_with_special_chars() {
    let tmp = TempDir::new().unwrap();
    let result = CommandSandbox::route_command(
        "echo 'hello world & !@#'",
        true,
        tmp.path().to_str().unwrap(),
    );
    assert!(result.is_ok());
    assert!(result.unwrap().contains("hello world"));
}
