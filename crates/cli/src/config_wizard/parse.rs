//! Model selection parsing helpers for the config wizard.

use closeclaw_llm::model_info::ModelInfo;
use closeclaw_llm::DiscoverySource;

/// Format a human-readable source suffix for the discovered model list.
///
/// Returns a string like `" (via knowledge base fallback)"` for
/// `KnowledgeFallback`, or an empty string for other sources.
pub(crate) fn format_discovery_source_suffix(source: DiscoverySource) -> &'static str {
    match source {
        DiscoverySource::KnowledgeFallback => " (via knowledge base fallback)",
        _ => "",
    }
}

/// Parse user input for model selection into a vector of 0-based indices.
///
/// Supported formats:
/// - Space-separated numbers: "1 3 5"
/// - Range syntax: "1-3"
/// - Mixed: "1-3,5,7"
/// - `all` keyword: selects all models
///
/// Returns a sorted, deduplicated vector of indices.
pub fn parse_model_selection(input: &str, total: usize) -> anyhow::Result<Vec<usize>> {
    let input = input.trim();
    if input.is_empty() {
        anyhow::bail!("selection cannot be empty");
    }

    if input.eq_ignore_ascii_case("all") {
        return Ok((0..total).collect());
    }

    let mut indices: Vec<usize> = Vec::new();

    // Normalize: replace spaces with commas, then split by comma.
    // This makes both "1 3 5" and "1-3,5,7" work.
    let normalized = input.replace(' ', ",");
    let parts: Vec<&str> = normalized
        .split(',')
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
        .collect();

    for part in parts {
        // Check for range syntax "n-m"
        if part.contains('-') {
            let range_parts: Vec<&str> = part.split('-').collect();
            if range_parts.len() != 2 {
                anyhow::bail!("invalid range syntax: '{}'", part);
            }
            let start: usize = range_parts[0]
                .trim()
                .parse()
                .map_err(|_| anyhow::anyhow!("invalid number: '{}'", range_parts[0].trim()))?;
            let end: usize = range_parts[1]
                .trim()
                .parse()
                .map_err(|_| anyhow::anyhow!("invalid number: '{}'", range_parts[1].trim()))?;
            if start == 0 || end == 0 {
                anyhow::bail!("model numbers are 1-based, 0 is not allowed");
            }
            if start > end {
                anyhow::bail!("invalid range: start ({}) > end ({})", start, end);
            }
            if start > total || end > total {
                anyhow::bail!(
                    "model number {} out of range (max is {})",
                    start.max(end),
                    total
                );
            }
            // Expand range "start-end" to all indices in [start, end]
            for idx in start..=end {
                indices.push(idx - 1);
            }
        } else {
            // Single number
            let n: usize = part
                .parse()
                .map_err(|_| anyhow::anyhow!("invalid number: '{}'", part))?;
            if n == 0 {
                anyhow::bail!("model numbers are 1-based, 0 is not allowed");
            }
            if n > total {
                anyhow::bail!("model number {} out of range (max is {})", n, total);
            }
            indices.push(n - 1);
        }
    }

    indices.sort_unstable();
    indices.dedup();
    Ok(indices)
}

/// Compute a default selection string for already-configured models.
///
/// Given a list of fetched models and a set of already-configured model IDs,
/// returns a comma-separated string of 1-based indices (e.g. "1,3,5")
/// for models that exist in `configured_ids`. Returns an empty string if no
/// models are already configured.
pub(crate) fn compute_default_selection(
    models: &[ModelInfo],
    configured_ids: &std::collections::HashSet<String>,
) -> String {
    let indices: Vec<String> = models
        .iter()
        .enumerate()
        .filter(|(_, m)| configured_ids.contains(&m.id))
        .map(|(i, _)| (i + 1).to_string())
        .collect();
    indices.join(",")
}
