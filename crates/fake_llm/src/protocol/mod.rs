//! Protocol-specific request parsing and response serialization.
//!
//! Each submodule handles one protocol (OpenAI or Anthropic) and is responsible for:
//! - Deserializing the protocol-specific request body
//! - Extracting protocol-agnostic `RequestFeatures`
//! - Serializing the response back into the protocol format
//!
//! Endpoint handlers delegate to these modules and only handle routing.

pub mod anthropic;
pub mod openai;
