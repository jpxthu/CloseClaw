//! CLI Tool - closeclaw command-line interface

pub mod admin;
pub mod args;
pub mod bridge;
pub mod chat;
pub mod config_wizard;
pub mod renderer;
pub mod terminal;

#[cfg(test)]
mod chat_tests;
#[cfg(test)]
mod renderer_blank_line_tests;
#[cfg(test)]
mod renderer_cjk_tests;
#[cfg(test)]
mod renderer_link_tests;
#[cfg(test)]
mod renderer_per_line_truncation_tests;
#[cfg(test)]
mod renderer_tests;
#[cfg(test)]
mod renderer_tool_result_tests;
#[cfg(test)]
mod terminal_tests;
