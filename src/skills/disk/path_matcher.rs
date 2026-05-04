//! PathMatcher — glob-based file path matching engine
//!
//! Wraps `glob::Pattern` to match file paths against a collection of
//! glob patterns. Used for conditional skill activation based on the
//! `paths` field in skill manifests.

use std::path::Path;

/// A collection of compiled glob patterns for matching file paths.
///
/// # Examples
/// ```ignore
/// let matcher = PathMatcher::new(&["**/*.rs".into()]).unwrap();
/// assert!(matcher.matches(Path::new("src/main.rs")));
/// ```
#[derive(Debug, Clone)]
pub struct PathMatcher {
    patterns: Vec<glob::Pattern>,
}

impl PathMatcher {
    /// Create a new PathMatcher from a slice of glob pattern strings.
    ///
    /// Returns an error if any pattern string is invalid glob syntax.
    pub fn new(patterns: &[String]) -> Result<Self, glob::PatternError> {
        let mut compiled = Vec::with_capacity(patterns.len());
        for p in patterns {
            compiled.push(glob::Pattern::new(p)?);
        }
        Ok(Self { patterns: compiled })
    }

    /// Check whether the given file path matches any of the stored patterns.
    ///
    /// Returns `false` when the pattern list is empty.
    pub fn matches(&self, path: &Path) -> bool {
        // Convert to string; glob::Pattern::matches works with &str.
        let path_str = path.to_string_lossy();
        self.patterns
            .iter()
            .any(|pattern| pattern.matches(&path_str))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn positive_match_rs_glob() {
        let matcher = PathMatcher::new(&["**/*.rs".into()]).unwrap();
        assert!(matcher.matches(Path::new("src/main.rs")));
    }

    #[test]
    fn positive_match_cargo_toml_glob() {
        let matcher = PathMatcher::new(&["**/Cargo.toml".into()]).unwrap();
        assert!(matcher.matches(Path::new("Cargo.toml")));
    }

    #[test]
    fn negative_match_rs_glob_against_ts() {
        let matcher = PathMatcher::new(&["**/*.rs".into()]).unwrap();
        assert!(!matcher.matches(Path::new("src/main.ts")));
    }

    #[test]
    fn empty_patterns_returns_false() {
        let matcher = PathMatcher::new(&[]).unwrap();
        assert!(!matcher.matches(Path::new("src/main.rs")));
        assert!(!matcher.matches(Path::new("any/path.txt")));
    }

    #[test]
    fn invalid_pattern_returns_error() {
        // Unmatched '[' is invalid glob syntax
        let result = PathMatcher::new(&["[invalid".into()]);
        assert!(result.is_err());
    }

    #[test]
    fn multi_pattern_match() {
        let matcher = PathMatcher::new(&["**/*.rs".into(), "**/*.toml".into()]).unwrap();
        // Matches via first pattern
        assert!(matcher.matches(Path::new("src/main.rs")));
        // Matches via second pattern
        assert!(matcher.matches(Path::new("Cargo.toml")));
        // Doesn't match any
        assert!(!matcher.matches(Path::new("src/main.ts")));
    }

    #[test]
    fn single_pattern_multiple_paths() {
        let matcher = PathMatcher::new(&["docs/**/*.md".into()]).unwrap();
        assert!(matcher.matches(Path::new("docs/README.md")));
        assert!(matcher.matches(Path::new("docs/developer/style.md")));
        assert!(!matcher.matches(Path::new("README.md")));
    }
}
