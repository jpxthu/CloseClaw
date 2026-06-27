use super::*;

fn analyzer() -> BashSecurityAnalyzer {
    BashSecurityAnalyzer::new().expect("analyzer creation failed")
}

#[test]
fn test_echo_hello() {
    let r = analyzer().analyze("echo hello");
    assert_eq!(r.commands.len(), 1);
    assert_eq!(r.commands[0].argv, vec!["echo", "hello"]);
    assert_eq!(r.trust_level, TrustLevel::Trusted);
}

#[test]
fn test_git_commit() {
    let r = analyzer().analyze("git commit -m \"fix bug\"");
    assert_eq!(r.commands.len(), 1);
    assert!(r.commands[0].argv[0] == "git");
    assert!(r.commands[0].argv.contains(&"commit".to_string()));
    assert_eq!(r.trust_level, TrustLevel::Trusted);
}

#[test]
fn test_pipe() {
    let r = analyzer().analyze("ls | wc -l");
    assert_eq!(r.commands.len(), 2);
    assert_eq!(r.commands[0].argv[0], "ls");
    assert_eq!(r.commands[1].argv[0], "wc");
}

#[test]
fn test_syntax_error() {
    let r = analyzer().analyze("if then");
    assert_eq!(r.trust_level, TrustLevel::Uncertain);
    assert!(r.reason.is_some());
}

#[test]
fn test_redirect() {
    let r = analyzer().analyze("echo hello > out.txt");
    assert_eq!(r.commands[0].redirects[0].operator, ">");
    assert_eq!(r.commands[0].redirects[0].target, "out.txt");
}

#[test]
fn test_empty_input() {
    assert!(analyzer().analyze("").commands.is_empty());
}

#[test]
fn test_multiple_commands_semicolon() {
    assert!(analyzer().analyze("echo a; echo b").commands.len() >= 2);
}

// Step 1.2 tests

#[test]
fn test_trusted_simple_command() {
    assert_eq!(
        analyzer().analyze("ls -la /tmp").trust_level,
        TrustLevel::Trusted
    );
}

#[test]
fn test_ansi_c_quoting_uncertain() {
    let r = analyzer().analyze("echo $'\\x63'");
    assert_eq!(r.trust_level, TrustLevel::Uncertain);
    assert!(r.reason.unwrap().contains("ANSI-C"));
}

#[test]
fn test_brace_expansion_uncertain() {
    let r = analyzer().analyze("echo {a,b}");
    assert_eq!(r.trust_level, TrustLevel::Uncertain);
    assert!(r.reason.unwrap().contains("Brace"));
}

#[test]
fn test_unquoted_redirect_uncertain() {
    let r = analyzer().analyze("echo hi > $FILE");
    assert_eq!(r.trust_level, TrustLevel::Uncertain);
    assert!(r.reason.unwrap().contains("Unquoted redirect"));
}

#[test]
fn test_ifs_injection_malicious() {
    let r = analyzer().analyze("IFS=x read line");
    assert_eq!(r.trust_level, TrustLevel::Malicious);
    assert!(r.reason.unwrap().contains("IFS"));
}

#[test]
fn test_ifs_dollar_malicious() {
    assert_eq!(
        analyzer().analyze("echo $IFS").trust_level,
        TrustLevel::Malicious
    );
}

#[test]
fn test_variable_as_command_uncertain() {
    assert_eq!(
        analyzer().analyze("CMD=ls; $CMD").trust_level,
        TrustLevel::Uncertain
    );
}

#[test]
fn test_interpret_exit_code() {
    assert_eq!(
        interpret_exit_code("grep pattern file", 0),
        Some("match found".into())
    );
    assert_eq!(
        interpret_exit_code("grep pattern file", 1),
        Some("no match found".into())
    );
    assert_eq!(
        interpret_exit_code("diff a b", 0),
        Some("files are identical".into())
    );
    assert_eq!(
        interpret_exit_code("diff a b", 1),
        Some("files differ".into())
    );
    assert_eq!(interpret_exit_code("echo hi", 0), None);
}
