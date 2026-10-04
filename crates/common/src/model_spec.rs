//! ModelSpec — agent model specification with optional fallback list.

use serde::de::{self, Deserializer, MapAccess, Visitor};
use serde::ser::SerializeStruct;
use serde::{Deserialize, Serialize};
use std::fmt;

/// Agent model specification with optional fallback list.
///
/// Supports two JSON formats for backward compatibility:
/// - String: `"gpt-4o"` → single model, no fallback
/// - Object: `{"primary": "gpt-4o", "fallback": ["claude-3"]}` → with fallback list
///
/// The primary model is always the first to try. Fallback models are tried
/// in order if the primary is unavailable; actual fallback logic lives in
/// the LLM layer (`unified_fallback.rs`), not here.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ModelSpec {
    /// Primary model identifier, tried first (e.g. `"gpt-4o"`).
    pub primary: String,
    /// Fallback model identifiers, tried in order when the primary is unavailable.
    /// Empty when the spec was created from the string form.
    pub fallback: Vec<String>,
}

impl ModelSpec {
    /// Create a ModelSpec with a single primary model and no fallbacks.
    pub fn single(model: impl Into<String>) -> Self {
        Self {
            primary: model.into(),
            fallback: Vec::new(),
        }
    }

    /// Create a ModelSpec with a primary model and a list of fallbacks.
    pub fn with_fallback(primary: impl Into<String>, fallback: Vec<String>) -> Self {
        Self {
            primary: primary.into(),
            fallback,
        }
    }
}

impl fmt::Display for ModelSpec {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.primary)
    }
}

impl Serialize for ModelSpec {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        if self.fallback.is_empty() {
            serializer.serialize_str(&self.primary)
        } else {
            let mut state = serializer.serialize_struct("ModelSpec", 2)?;
            state.serialize_field("primary", &self.primary)?;
            state.serialize_field("fallback", &self.fallback)?;
            state.end()
        }
    }
}

struct ModelSpecVisitor;

impl<'de> Visitor<'de> for ModelSpecVisitor {
    type Value = ModelSpec;

    fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
        formatter.write_str("a model name string or {primary, fallback} object")
    }

    fn visit_str<E: de::Error>(self, value: &str) -> Result<ModelSpec, E> {
        Ok(ModelSpec::single(value))
    }

    fn visit_string<E: de::Error>(self, value: String) -> Result<ModelSpec, E> {
        Ok(ModelSpec::single(value))
    }

    fn visit_map<M: MapAccess<'de>>(self, mut map: M) -> Result<ModelSpec, M::Error> {
        let mut primary = None;
        let mut fallback = None;

        while let Some(key) = map.next_key::<String>()? {
            match key.as_str() {
                "primary" => {
                    if primary.is_some() {
                        return Err(de::Error::duplicate_field("primary"));
                    }
                    primary = Some(map.next_value()?);
                }
                "fallback" => {
                    if fallback.is_some() {
                        return Err(de::Error::duplicate_field("fallback"));
                    }
                    fallback = Some(map.next_value()?);
                }
                _ => {
                    let _ = map.next_value::<de::IgnoredAny>()?;
                }
            }
        }

        let primary = primary.ok_or_else(|| de::Error::missing_field("primary"))?;
        let fallback = fallback.unwrap_or_default();

        Ok(ModelSpec { primary, fallback })
    }
}

impl<'de> Deserialize<'de> for ModelSpec {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        deserializer.deserialize_any(ModelSpecVisitor)
    }
}
