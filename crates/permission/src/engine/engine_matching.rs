//! Permission Engine - Glob and action matching utilities.

use super::engine_types::{Action, CommandArgs, PermissionRequestBody};

/// Simple glob matching (supports * and **)
pub fn glob_match(pattern: &str, text: &str) -> bool {
    if pattern == "**" || pattern == "*" {
        return true;
    }

    let pattern_chars: Vec<char> = pattern.chars().collect();
    let text_chars: Vec<char> = text.chars().collect();

    glob_match_vec(&pattern_chars, &text_chars, 0, 0)
}

fn glob_match_vec(pat: &[char], text: &[char], pi: usize, ti: usize) -> bool {
    if pi == pat.len() && ti == text.len() {
        return true;
    }

    if pi < pat.len() && pat[pi] == '*' {
        if pi + 1 < pat.len() && pat[pi + 1] == '*' {
            if pi + 2 < pat.len() && (pat[pi + 2] == '/' || pat[pi + 2] == '\\') {
                if pi + 3 < pat.len() {
                    return glob_match_vec(pat, text, pi + 3, ti)
                        || (ti < text.len() && glob_match_vec(pat, text, pi, ti + 1));
                }
                return ti >= text.len() || text[ti] == '/';
            }
            return ti >= text.len()
                || glob_match_vec(pat, text, pi + 2, ti)
                || glob_match_vec(pat, text, pi, ti + 1);
        }
        if ti >= text.len() {
            return glob_match_vec(pat, text, pi + 1, ti);
        }
        return text[ti] != '/'
            && (glob_match_vec(pat, text, pi + 1, ti) || glob_match_vec(pat, text, pi, ti + 1));
    }

    if pi < pat.len() && ti < text.len() && (pat[pi] == '?' || pat[pi] == text[ti]) {
        return glob_match_vec(pat, text, pi + 1, ti + 1);
    }

    false
}

/// Check if a single action matches the request.
pub fn action_matches_request(action: &Action, request: &PermissionRequestBody) -> bool {
    if matches!(action, Action::All) {
        return true;
    }

    match (action, request) {
        (Action::File { operation, paths }, PermissionRequestBody::FileOp { path, op, .. }) => {
            operation == op && paths.iter().any(|p| glob_match(p, path))
        }
        (
            Action::Command { command, args },
            PermissionRequestBody::CommandExec {
                cmd,
                args: req_args,
                ..
            },
        ) => {
            if command != cmd {
                return false;
            }
            match args {
                CommandArgs::Any => true,
                CommandArgs::Allowed { allowed } => req_args
                    .iter()
                    .all(|arg| allowed.iter().any(|a| glob_match(a, arg))),
                CommandArgs::Blocked { blocked } => req_args
                    .iter()
                    .any(|arg| blocked.iter().any(|b| glob_match(b, arg))),
            }
        }
        (Action::Network { hosts, ports }, PermissionRequestBody::NetOp { host, port, .. }) => {
            (hosts.is_empty() || hosts.iter().any(|h| glob_match(h, host)))
                && (ports.is_empty() || ports.contains(port))
        }
        (
            Action::ToolCall { skill, methods },
            PermissionRequestBody::ToolCall {
                skill: s, method, ..
            },
        ) => skill == s && (methods.is_empty() || methods.contains(method)),
        (Action::InterAgent { agents }, PermissionRequestBody::InterAgentMsg { to, .. }) => {
            agents.is_empty() || agents.iter().any(|a| glob_match(a, to))
        }
        (Action::ConfigWrite { files }, PermissionRequestBody::ConfigWrite { config_file, .. }) => {
            files.is_empty() || files.iter().any(|f| glob_match(f, config_file))
        }
        _ => false,
    }
}
