//! Security analysis module for Bash commands.
//!
//! Provides AST-based parsing via tree-sitter and trust-level classification.

pub mod bash_analyzer;

pub use bash_analyzer::{
    interpret_exit_code, BashSecurityAnalyzer, ParseResult, Redirect, SimpleCommand, TrustLevel,
};
