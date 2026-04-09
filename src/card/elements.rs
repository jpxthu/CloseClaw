//! Card element data structures for Feishu interactive cards.

use serde::{Deserialize, Serialize};

/// Card element enum — each variant maps to a Feishu card element type.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum CardElement {
    /// Markdown text block
    Markdown(MarkdownElement),
    /// Progress bar
    Progress(ProgressElement),
    /// Button
    Button(ButtonElement),
    /// Divider
    Divider,
    /// Image
    Image(ImageElement),
}

/// Markdown text block
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MarkdownElement {
    /// Supported markdown: bold, italic, code, list, link
    pub content: String,
    /// Whether the block can be collapsed
    #[serde(default)]
    pub collapsible: bool,
    /// Default collapsed state
    #[serde(default)]
    pub collapsed: bool,
}

/// Progress bar element
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProgressElement {
    /// Current step (1-indexed)
    pub current: u32,
    /// Total number of steps
    pub total: u32,
    /// Optional labels for each step
    pub labels: Option<Vec<String>>,
}

/// Button element
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ButtonElement {
    pub text: String,
    pub action: CardAction,
    pub style: ButtonStyle,
}

/// Button style
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ButtonStyle {
    Primary,
    Secondary,
    Default,
}

/// Card action — triggered when a user clicks a button.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "action", rename_all = "snake_case")]
pub enum CardAction {
    /// Expand a step (show its content)
    ExpandStep { step_index: u32 },
    /// Collapse a step (hide its content)
    CollapseStep { step_index: u32 },
    /// Confirm the plan
    Confirm,
    /// Cancel the plan
    Cancel,
    /// Custom action with a string payload
    Custom { payload: String },
}

/// Image element
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImageElement {
    /// Image URL or img_key
    pub url: String,
    /// Alt text
    pub alt: Option<String>,
}
