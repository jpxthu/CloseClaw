//! Bash security analyzer using tree-sitter AST parsing + trust grading.

use tree_sitter::{Node, Parser};

/// Node kinds considered safe standard shell syntax.
const TRUSTED_NODE_KINDS: &[&str] = &[
    "program",
    "command",
    "pipeline",
    "list",
    "redirected_statement",
    "file_redirect",
    "heredoc_redirect",
    "heredoc_body",
    "variable_assignment",
    "subshell",
    "compound_statement",
    "command_name",
    "word",
    "string",
    "string_content",
    "raw_string",
    "ansi_c_string",
    "simple_expansion",
    "variable_name",
    "$",
    "concatenation",
    "file_descriptor",
    "if_statement",
    "for_statement",
    "while_statement",
    "case_statement",
    "function_definition",
    "case_item",
    "do_group",
    "else_clause",
    "elif_clause",
    "test_command",
    "negated_command",
    ";",
    "\n",
    "|",
    "&&",
    "||",
    "<",
    ">",
    ">>",
    "&>",
    "&>>",
    "<&",
    ">&",
    "\"",
];

fn is_trusted_kind(kind: &str) -> bool {
    TRUSTED_NODE_KINDS.contains(&kind)
}

// -- Data models --

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TrustLevel {
    Trusted,
    Uncertain,
    Malicious,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Redirect {
    pub fd: Option<String>,
    pub operator: String,
    pub target: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SimpleCommand {
    pub argv: Vec<String>,
    pub env_vars: Vec<(String, String)>,
    pub redirects: Vec<Redirect>,
    pub source_text: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParseResult {
    pub commands: Vec<SimpleCommand>,
    pub trust_level: TrustLevel,
    pub reason: Option<String>,
}

// -- Analyzer --

pub struct BashSecurityAnalyzer {
    parser: Parser,
}

impl BashSecurityAnalyzer {
    pub fn new() -> Result<Self, String> {
        let mut parser = Parser::new();
        let lang = tree_sitter_bash::LANGUAGE.into();
        parser
            .set_language(&lang)
            .map_err(|e| format!("Failed to set bash language: {e}"))?;
        Ok(Self { parser })
    }

    pub fn analyze(&mut self, source: &str) -> ParseResult {
        let tree = self.parser.parse(source, None);
        match tree {
            Some(tree) => {
                let root = tree.root_node();
                if root.has_error() {
                    return uncertain_result(collect_error_text(&root, source));
                }
                let commands = extract_commands(&root, source);
                // Trust grading: malicious → unknown nodes → diff detectors
                if let Some(r) = detect_malicious(source) {
                    return ParseResult {
                        commands,
                        trust_level: TrustLevel::Malicious,
                        reason: Some(r),
                    };
                }
                let unknown = collect_unknown_kinds(&root);
                if !unknown.is_empty() {
                    return ParseResult {
                        commands,
                        trust_level: TrustLevel::Uncertain,
                        reason: Some(format!("Unknown node types: {}", unknown.join(", "))),
                    };
                }
                if let Some(r) = run_diff_detectors(source) {
                    return ParseResult {
                        commands,
                        trust_level: TrustLevel::Uncertain,
                        reason: Some(r),
                    };
                }
                ParseResult {
                    commands,
                    trust_level: TrustLevel::Trusted,
                    reason: None,
                }
            }
            None => uncertain_result("Parser returned no tree (possibly timeout)".into()),
        }
    }
}

impl Default for BashSecurityAnalyzer {
    fn default() -> Self {
        Self::new().expect("BashSecurityAnalyzer init should not fail")
    }
}

// -- Command extraction (module-level functions) --

fn uncertain_result(reason: String) -> ParseResult {
    ParseResult {
        commands: vec![],
        trust_level: TrustLevel::Uncertain,
        reason: Some(reason),
    }
}

fn extract_commands(root: &Node, source: &str) -> Vec<SimpleCommand> {
    let mut out = Vec::new();
    walk_for_commands(root, source, &mut out);
    out
}

fn walk_for_commands<'a>(node: &Node<'a>, source: &str, out: &mut Vec<SimpleCommand>) {
    match node.kind() {
        "command" => {
            if let Some(cmd) = build_simple_command(node, source) {
                out.push(cmd);
            }
        }
        "redirected_statement" => handle_redirected(node, source, out),
        _ => {
            let mut w = node.walk();
            for ch in node.children(&mut w) {
                walk_for_commands(&ch, source, out);
            }
        }
    }
}

fn handle_redirected<'a>(node: &Node<'a>, source: &str, out: &mut Vec<SimpleCommand>) {
    let mut cmd_opt: Option<SimpleCommand> = None;
    let mut redirects = Vec::new();
    let mut w = node.walk();
    for ch in node.children(&mut w) {
        match ch.kind() {
            "command" => cmd_opt = build_simple_command(&ch, source),
            "file_redirect" | "heredoc_redirect" => extract_redirect(&ch, source, &mut redirects),
            _ => {}
        }
    }
    if let Some(mut cmd) = cmd_opt {
        cmd.redirects = redirects;
        out.push(cmd);
    }
}

fn build_simple_command<'a>(node: &Node<'a>, source: &str) -> Option<SimpleCommand> {
    let mut argv = Vec::new();
    let mut env_vars = Vec::new();
    let mut redirects = Vec::new();
    let mut w = node.walk();
    for ch in node.children(&mut w) {
        match ch.kind() {
            "command_name" | "word" | "string" | "raw_string" | "simple_expansion"
            | "concatenation" => {
                if let Some(t) = node_text(&ch, source) {
                    argv.push(t);
                }
            }
            "variable_assignment" => extract_var_assign(&ch, source, &mut env_vars),
            "file_redirect" | "heredoc_redirect" | "redirect" => {
                extract_redirect(&ch, source, &mut redirects)
            }
            _ => {}
        }
    }
    let source_text = node_text(node, source).unwrap_or_default();
    if argv.is_empty() {
        None
    } else {
        Some(SimpleCommand {
            argv,
            env_vars,
            redirects,
            source_text,
        })
    }
}

fn extract_var_assign<'a>(node: &Node<'a>, source: &str, env_vars: &mut Vec<(String, String)>) {
    let t = node_text(node, source).unwrap_or_default();
    if let Some((k, v)) = t.split_once('=') {
        env_vars.push((k.into(), v.into()));
    }
}

fn extract_redirect<'a>(node: &Node<'a>, source: &str, redirects: &mut Vec<Redirect>) {
    let (mut fd, mut op, mut tgt) = (None, String::new(), String::new());
    let mut w = node.walk();
    for ch in node.children(&mut w) {
        match ch.kind() {
            "file_descriptor" => fd = node_text(&ch, source),
            "<" | ">" | ">>" | "&>" | "&>>" | "<&" | ">&" => op = ch.kind().to_string(),
            "word" | "string" | "simple_expansion" => {
                tgt = node_text(&ch, source).unwrap_or_default()
            }
            _ => {}
        }
    }
    if !op.is_empty() {
        redirects.push(Redirect {
            fd,
            operator: op,
            target: tgt,
        });
    }
}

fn node_text(node: &Node, source: &str) -> Option<String> {
    let (s, e) = (node.start_byte(), node.end_byte());
    (s <= e && e <= source.len()).then(|| source[s..e].to_string())
}

fn collect_error_text<'a>(root: &Node<'a>, source: &str) -> String {
    let mut errs = Vec::new();
    find_errors(root, source, &mut errs);
    if errs.is_empty() {
        "Syntax error in command".into()
    } else {
        errs.join("; ")
    }
}

fn find_errors<'a>(node: &Node<'a>, source: &str, errors: &mut Vec<String>) {
    if node.is_error() || node.is_missing() {
        let t = node_text(node, source).unwrap_or_else(|| node.kind().to_string());
        let label = if node.is_missing() {
            "Missing node"
        } else {
            "Error node"
        };
        errors.push(format!("{label} at row {}: {t}", node.start_position().row));
    }
    let mut w = node.walk();
    for ch in node.children(&mut w) {
        find_errors(&ch, source, errors);
    }
}

// -- Trust whitelist + diff detectors --

fn collect_unknown_kinds(root: &Node) -> Vec<String> {
    let mut out = Vec::new();
    walk_unknown(root, &mut out);
    out.sort();
    out.dedup();
    out
}

fn walk_unknown<'a>(node: &Node<'a>, out: &mut Vec<String>) {
    let kind = node.kind();
    if !kind.is_empty()
        && !node.is_extra()
        && !is_trusted_kind(kind)
        && !out.contains(&kind.to_string())
    {
        out.push(kind.to_string());
    }
    let mut w = node.walk();
    for ch in node.children(&mut w) {
        walk_unknown(&ch, out);
    }
}

fn detect_malicious(source: &str) -> Option<String> {
    if detect_ifs_injection(source) {
        return Some("IFS injection detected".into());
    }
    None
}

fn run_diff_detectors(source: &str) -> Option<String> {
    if detect_ansi_c_quoting(source) {
        return Some("ANSI-C quoting detected ($'...')".into());
    }
    if detect_brace_expansion(source) {
        return Some("Brace expansion detected".into());
    }
    if detect_unquoted_redirect(source) {
        return Some("Unquoted redirect target".into());
    }
    None
}

/// Detect `$'...'` (ANSI-C quoting).
fn detect_ansi_c_quoting(source: &str) -> bool {
    let b = source.as_bytes();
    (0..b.len().saturating_sub(1)).any(|i| b[i] == b'$' && b[i + 1] == b'\'')
}

/// Detect brace expansion like `{a,b}` or `{1..5}`.
fn detect_brace_expansion(source: &str) -> bool {
    for i in 0..source.len() {
        if source.as_bytes()[i] == b'{' {
            if let Some(end) = source[i..].find('}') {
                let inner = &source[i + 1..i + end];
                if inner.contains(',') || inner.contains("..") {
                    return true;
                }
            }
        }
    }
    false
}

/// Detect unquoted redirect targets (e.g. `> $VAR`).
fn detect_unquoted_redirect(source: &str) -> bool {
    regex_lite::Regex::new(r"[<>]{1,2}\s*\$")
        .map(|r| r.is_match(source))
        .unwrap_or(false)
}

/// Detect IFS injection: `IFS=...` with suspicious values or `$IFS`.
fn detect_ifs_injection(source: &str) -> bool {
    source.split_ascii_whitespace().any(|w| {
        w.starts_with("IFS=")
            && w.len() > 4
            && !w[4..].starts_with("$'\t\n'")
            && !w[4..].starts_with("'\t\n'")
    }) || source.contains("$IFS")
}

/// Extract the first word of a command string for dispatch.
fn first_word(segment: &str) -> &str {
    segment.split_whitespace().next().unwrap_or("")
}

/// Interpret well-known exit codes for common CLI tools.
pub fn interpret_exit_code(command: &str, exit_code: i32) -> Option<String> {
    let word = first_word(command);
    match word {
        "grep" => match exit_code {
            0 => Some("match found".into()),
            1 => Some("no match found".into()),
            _ => None,
        },
        "diff" => match exit_code {
            0 => Some("files are identical".into()),
            1 => Some("files differ".into()),
            _ => None,
        },
        "find" => match exit_code {
            0 => Some("matches found".into()),
            1 => Some("no matches found".into()),
            _ => None,
        },
        _ => None,
    }
}

#[cfg(test)]
#[path = "bash_analyzer_tests.rs"]
mod tests;
