//! Card module — Feishu interactive card system for Plan mode.
//!
//! Provides data structures, builders, renderers, and event handlers for
//! rich Feishu cards with progress bars, step lists, and interactive buttons.

use serde::{Deserialize, Serialize};

pub mod builder;
pub mod elements;
pub mod events;
pub mod handler;
pub mod renderer;
pub mod update;

// Re-export commonly used types
pub use elements::{CardAction, CardElement};
pub use elements::{ButtonElement, ButtonStyle, ImageElement, MarkdownElement, ProgressElement};
pub use renderer::render_feishu_card;

/// Card header configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CardHeader {
    pub title: String,
    pub subtitle: Option<String>,
    pub avatar_url: Option<String>,
}

/// Rich text card for Feishu interactive messages
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RichCard {
    /// Feishu message ID — used for subsequent card updates
    pub card_id: Option<String>,
    /// Card title
    pub title: String,
    /// Ordered list of card elements
    pub elements: Vec<CardElement>,
    /// Optional card header
    pub header: Option<CardHeader>,
}

/// Plan mode card data
#[derive(Debug, Clone)]
pub struct PlanData {
    pub title: String,
    pub current_step: u32,
    pub total_steps: u32,
    pub step_labels: Vec<String>,
    pub steps: Vec<PlanStep>,
    pub is_high_complexity: bool,
}

/// A single step within a Plan
#[derive(Debug, Clone)]
pub struct PlanStep {
    pub title: String,
    pub content: String,
    pub status: StepStatus,
}

/// Step execution status
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StepStatus {
    Pending,
    Active,
    Completed,
}
