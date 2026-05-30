//! Bash command classification for UI display optimization.
//!
//! Classifies commands by their first word into categories:
//! - search: grep, find, rg, ag, ack, locate, which, whereis
//! - read: cat, head, tail, less, more, wc, jq, awk
//! - list: ls, tree, du
//! - silent: mv, cp, rm, mkdir, rmdir, chmod, chown, chgrp, touch, ln, cd, export, unset, wait
//! - neutral: echo, printf, true, false, :

/// Command category for semantic classification.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CommandCategory {
    Search,
    Read,
    List,
    Silent,
    Neutral,
    Unknown,
}

/// Classify a single command word into a category.
fn classify_word(word: &str) -> CommandCategory {
    match word {
        "grep" | "find" | "rg" | "ag" | "ack" | "locate" | "which" | "whereis" => {
            CommandCategory::Search
        }
        "cat" | "head" | "tail" | "less" | "more" | "wc" | "jq" | "awk" => CommandCategory::Read,
        "ls" | "tree" | "du" => CommandCategory::List,
        "mv" | "cp" | "rm" | "mkdir" | "rmdir" | "chmod" | "chown" | "chgrp" | "touch" | "ln"
        | "cd" | "export" | "unset" | "wait" => CommandCategory::Silent,
        "echo" | "printf" | "true" | "false" | ":" => CommandCategory::Neutral,
        _ => CommandCategory::Unknown,
    }
}

/// Extract the first word from a command segment (ignores leading whitespace).
fn first_word(segment: &str) -> &str {
    segment.split_whitespace().next().unwrap_or("")
}

/// Check if a category belongs to the foldable set.
fn is_foldable(cat: CommandCategory) -> bool {
    matches!(
        cat,
        CommandCategory::Search
            | CommandCategory::Read
            | CommandCategory::List
            | CommandCategory::Neutral
    )
}

/// Classify a command string by its first word.
///
/// For pipe commands (contains `|`), classifies each segment.
/// Returns `Unknown` if any segment is not in
/// {search, read, list, neutral}.
pub(crate) fn classify_command(command: &str) -> CommandCategory {
    if !command.contains('|') {
        return classify_word(first_word(command));
    }
    // Pipe command: all segments must be foldable; return first segment's category.
    let mut first_cat = CommandCategory::Unknown;
    for (i, segment) in command.split('|').enumerate() {
        let cat = classify_word(first_word(segment));
        if !is_foldable(cat) {
            return CommandCategory::Unknown;
        }
        if i == 0 {
            first_cat = cat;
        }
    }
    first_cat
}

/// Returns true if the command is expected to produce no output
/// on success.
pub(crate) fn no_output_expected(category: CommandCategory) -> bool {
    matches!(category, CommandCategory::Silent)
}

/// Returns a semantic interpretation of the exit code for known
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_classify_single_commands() {
        assert_eq!(
            classify_command("grep pattern file"),
            CommandCategory::Search
        );
        assert_eq!(classify_command("cat file.txt"), CommandCategory::Read);
        assert_eq!(classify_command("ls -la"), CommandCategory::List);
        assert_eq!(classify_command("mv a b"), CommandCategory::Silent);
        assert_eq!(classify_command("echo hello"), CommandCategory::Neutral);
        assert_eq!(classify_command("docker ps"), CommandCategory::Unknown);
    }

    #[test]
    fn test_pipe_commands_foldable() {
        assert_eq!(classify_command("grep x | wc -l"), CommandCategory::Search);
        assert_eq!(
            classify_command("cat file | head -5"),
            CommandCategory::Read
        );
    }

    #[test]
    fn test_pipe_commands_non_foldable() {
        assert_eq!(
            classify_command("grep x | mv a b"),
            CommandCategory::Unknown
        );
    }

    #[test]
    fn test_no_output_expected() {
        assert!(no_output_expected(CommandCategory::Silent));
        assert!(!no_output_expected(CommandCategory::Search));
        assert!(!no_output_expected(CommandCategory::Read));
        assert!(!no_output_expected(CommandCategory::List));
        assert!(!no_output_expected(CommandCategory::Neutral));
        assert!(!no_output_expected(CommandCategory::Unknown));
    }
}
