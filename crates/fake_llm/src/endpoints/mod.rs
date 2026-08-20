//! Protocol-specific HTTP endpoint handlers.
//!
//! Each submodule handles one endpoint and is responsible for:
//! - Parsing the protocol-specific request body
//! - Extracting protocol-agnostic `RequestFeatures`
//! - Delegating to the scenario engine (future) and returning a response

pub mod anthropic_messages;
pub mod models;
pub mod openai_chat;
