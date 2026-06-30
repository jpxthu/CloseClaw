//! CLI Tool - closeclaw command-line interface

pub mod admin;
pub mod args;
pub mod chat;
pub mod config_wizard;
pub mod renderer;
pub mod terminal;

#[cfg(test)]
mod chat_tests;
#[cfg(test)]
mod renderer_tests;
#[cfg(test)]
mod terminal_tests;
