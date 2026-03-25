//! Permission template system
//!
//! Templates provide reusable fragments of permission rules that can be
//! inherited and composed by actual rules via the `template` field.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::Path;

/// A permission template — a named, reusable fragment of rules.
/// Templates are stored as standalone files under templates/ and
/// can be inherited/composed by actual rules.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Template {
    /// Unique template name (used in TemplateRef).
    pub name: String,
    /// Human-readable description.
    #[serde(default)]
    pub description: String,
    /// The base subject pattern for this template.
    /// Resolved at composition time using the calling rule's context.
    pub subject: TemplateSubject,
    /// Default effect if not overridden.
    #[serde(default)]
    pub effect: crate::permission::engine::Effect,
    /// List of action specifications.
    pub actions: Vec<crate::permission::engine::Action>,
    /// Templates this template extends (single inheritance only).
    #[serde(default)]
    pub extends: Vec<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum TemplateSubject {
    /// Matches any caller; subject is provided by the composing rule.
    Any,
    /// Fixed agent pattern.
    Agent {
        agent: String,
        match_type: crate::permission::engine::MatchType,
    },
    /// Fixed user+agent pattern.
    UserAndAgent {
        user_id: String,
        agent: String,
        user_match: crate::permission::engine::MatchType,
        agent_match: crate::permission::engine::MatchType,
    },
}

/// Load all template files from a directory.
///
/// Reads all `.json` files under `config_dir/templates/` and parses them
/// as [`Template`] structs. Templates are returned as a map from template
/// name to the resolved (inheritance-expanded) template.
pub fn load_templates_from_dir(
    config_dir: &Path,
) -> Result<HashMap<String, Template>, TemplateLoadError> {
    let templates_dir = config_dir.join("templates");
    if !templates_dir.is_dir() {
        return Ok(HashMap::new());
    }

    let mut templates: HashMap<String, Template> = HashMap::new();

    let entries = std::fs::read_dir(&templates_dir)
        .map_err(|e| TemplateLoadError::IoError(templates_dir.clone(), e))?;

    let mut file_names: Vec<_> = entries
        .filter_map(|e| e.ok())
        .filter(|e| e.path().extension().map_or(false, |ext| ext == "json"))
        .map(|e| e.path())
        .collect();

    file_names.sort();

    for path in &file_names {
        let content = std::fs::read_to_string(path)
            .map_err(|e| TemplateLoadError::IoError(path.clone(), e))?;
        let tmpl: Template = serde_json::from_str(&content)
            .map_err(|e| TemplateLoadError::ParseError(path.clone(), e))?;
        templates.insert(tmpl.name.clone(), tmpl);
    }

    // Expand inheritance for all templates (single inheritance)
    expand_inheritance(&mut templates)?;

    Ok(templates)
}

/// Resolve a template by name, recursively expanding its inheritance chain.
/// Returns the fully-expanded template with all inherited actions merged.
pub(crate) fn expand_inheritance(
    templates: &mut HashMap<String, Template>,
) -> Result<(), TemplateLoadError> {
    // Topological sort to detect cycles and resolve in correct order
    let names: Vec<String> = templates.keys().cloned().collect();
    let mut resolved: HashMap<String, Template> = HashMap::new();
    let mut visiting: std::collections::HashSet<String> = std::collections::HashSet::new();
    let mut visited: std::collections::HashSet<String> = std::collections::HashSet::new();

    for name in &names {
        if !visited.contains(name) {
            resolve_template(name, templates, &mut resolved, &mut visiting, &mut visited)?;
        }
    }

    *templates = resolved;
    Ok(())
}

fn resolve_template(
    name: &str,
    source: &HashMap<String, Template>,
    resolved: &mut HashMap<String, Template>,
    visiting: &mut std::collections::HashSet<String>,
    visited: &mut std::collections::HashSet<String>,
) -> Result<Template, TemplateLoadError> {
    if let Some(t) = resolved.get(name) {
        return Ok(t.clone());
    }

    if !visiting.insert(name.to_string()) {
        return Err(TemplateLoadError::CycleDetected(name.to_string()));
    }

    let tmpl = source
        .get(name)
        .ok_or_else(|| TemplateLoadError::TemplateNotFound(name.to_string()))?
        .clone();

    // Recursively resolve parent templates
    let mut merged_actions = Vec::new();
    for parent_name in &tmpl.extends {
        let parent = resolve_template(parent_name, source, resolved, visiting, visited)?;
        merged_actions.extend(parent.actions);
    }
    merged_actions.extend(tmpl.actions);

    visiting.remove(name);
    visited.insert(name.to_string());

    let expanded = Template {
        name: tmpl.name.clone(),
        description: tmpl.description,
        subject: tmpl.subject,
        effect: tmpl.effect,
        actions: merged_actions,
        extends: vec![], // Flattened
    };

    resolved.insert(name.to_string(), expanded.clone());
    Ok(expanded)
}

/// Errors that can occur during template loading.
#[derive(Debug, thiserror::Error)]
pub enum TemplateLoadError {
    #[error("IO error reading template file: {0}")]
    IoError(std::path::PathBuf, #[source] std::io::Error),
    #[error("JSON parse error in template file: {0}")]
    ParseError(std::path::PathBuf, #[source] serde_json::Error),
    #[error("template not found: {0}")]
    TemplateNotFound(String),
    #[error("circular inheritance detected involving template: {0}")]
    CycleDetected(String),
}

#[cfg(test)]
mod tests {
    use super::*;

    fn developer_template() -> Template {
        Template {
            name: "developer".to_string(),
            description: "Standard development permissions".to_string(),
            subject: TemplateSubject::Agent {
                agent: "dev-*".to_string(),
                match_type: crate::permission::engine::MatchType::Glob,
            },
            effect: crate::permission::engine::Effect::Allow,
            actions: vec![
                crate::permission::engine::Action::File {
                    operation: "read".to_string(),
                    paths: vec!["**".to_string()],
                },
                crate::permission::engine::Action::File {
                    operation: "write".to_string(),
                    paths: vec!["/home/admin/code/**".to_string()],
                },
                crate::permission::engine::Action::Command {
                    command: "git".to_string(),
                    args: crate::permission::engine::CommandArgs::Allowed {
                        allowed: vec![
                            "status".to_string(),
                            "log".to_string(),
                            "diff".to_string(),
                            "add".to_string(),
                            "commit".to_string(),
                            "push".to_string(),
                            "pull".to_string(),
                        ],
                    },
                },
                crate::permission::engine::Action::Command {
                    command: "cargo".to_string(),
                    args: crate::permission::engine::CommandArgs::Any,
                },
            ],
            extends: vec![],
        }
    }

    fn readonly_template() -> Template {
        Template {
            name: "readonly".to_string(),
            description: "Read-only access to all resources.".to_string(),
            subject: TemplateSubject::Any,
            effect: crate::permission::engine::Effect::Allow,
            actions: vec![crate::permission::engine::Action::File {
                operation: "read".to_string(),
                paths: vec!["**".to_string()],
            }],
            extends: vec![],
        }
    }

    fn admin_template() -> Template {
        Template {
            name: "admin".to_string(),
            description: "Full access, inherits developer template and adds config write."
                .to_string(),
            subject: TemplateSubject::Agent {
                agent: "admin-*".to_string(),
                match_type: crate::permission::engine::MatchType::Glob,
            },
            effect: crate::permission::engine::Effect::Allow,
            actions: vec![crate::permission::engine::Action::ConfigWrite {
                files: vec!["**".to_string()],
            }],
            extends: vec!["developer".to_string()],
        }
    }

    #[test]
    fn test_template_deserialize() {
        let json = r#"{
            "name": "developer",
            "description": "Standard development permissions",
            "subject": { "type": "agent", "agent": "dev-*", "match_type": "glob" },
            "effect": "allow",
            "actions": [
                { "type": "file", "operation": "read", "paths": ["**"] }
            ],
            "extends": []
        }"#;
        let tmpl: Template = serde_json::from_str(json).unwrap();
        assert_eq!(tmpl.name, "developer");
        assert!(matches!(tmpl.subject, TemplateSubject::Agent { .. }));
    }

    #[test]
    fn test_template_subject_any_deserialize() {
        let json = r#"{
            "name": "readonly",
            "subject": { "type": "any" },
            "actions": [{ "type": "file", "operation": "read", "paths": ["**"] }]
        }"#;
        let tmpl: Template = serde_json::from_str(json).unwrap();
        assert!(matches!(tmpl.subject, TemplateSubject::Any));
    }

    #[test]
    fn test_template_subject_user_and_agent() {
        let json = r#"{
            "name": "user-dev",
            "subject": {
                "type": "user_and_agent",
                "user_id": "ou_123",
                "agent": "dev-*",
                "user_match": "exact",
                "agent_match": "glob"
            },
            "actions": [{ "type": "file", "operation": "read", "paths": ["**"] }]
        }"#;
        let tmpl: Template = serde_json::from_str(json).unwrap();
        assert!(matches!(tmpl.subject, TemplateSubject::UserAndAgent { .. }));
    }

    #[test]
    fn test_expand_inheritance_single() {
        let mut templates: HashMap<String, Template> = HashMap::new();
        templates.insert("readonly".to_string(), readonly_template());
        templates.insert("developer".to_string(), developer_template());
        templates.insert("admin".to_string(), admin_template());

        expand_inheritance(&mut templates).unwrap();

        let admin = templates.get("admin").unwrap();
        // Admin's own actions + inherited developer actions
        assert!(admin
            .actions
            .iter()
            .any(|a| matches!(a, crate::permission::engine::Action::ConfigWrite { .. })));
        assert!(admin.actions.iter().any(|a| matches!(a, crate::permission::engine::Action::Command { command, .. } if command == "cargo")));
    }

    #[test]
    fn test_expand_inheritance_no_cycle() {
        let mut templates: HashMap<String, Template> = HashMap::new();
        templates.insert(
            "a".to_string(),
            Template {
                name: "a".to_string(),
                description: "".to_string(),
                subject: TemplateSubject::Any,
                effect: crate::permission::engine::Effect::Allow,
                actions: vec![],
                extends: vec![],
            },
        );

        let result = expand_inheritance(&mut templates);
        assert!(result.is_ok());
    }

    #[test]
    fn test_expand_inheritance_detects_cycle() {
        let mut templates: HashMap<String, Template> = HashMap::new();
        templates.insert(
            "a".to_string(),
            Template {
                name: "a".to_string(),
                description: "".to_string(),
                subject: TemplateSubject::Any,
                effect: crate::permission::engine::Effect::Allow,
                actions: vec![],
                extends: vec!["b".to_string()],
            },
        );
        templates.insert(
            "b".to_string(),
            Template {
                name: "b".to_string(),
                description: "".to_string(),
                subject: TemplateSubject::Any,
                effect: crate::permission::engine::Effect::Allow,
                actions: vec![],
                extends: vec!["a".to_string()],
            },
        );

        let result = expand_inheritance(&mut templates);
        assert!(matches!(result, Err(TemplateLoadError::CycleDetected(_))));
    }

    #[test]
    fn test_expand_inheritance_missing_parent() {
        let mut templates: HashMap<String, Template> = HashMap::new();
        templates.insert(
            "child".to_string(),
            Template {
                name: "child".to_string(),
                description: "".to_string(),
                subject: TemplateSubject::Any,
                effect: crate::permission::engine::Effect::Allow,
                actions: vec![],
                extends: vec!["nonexistent".to_string()],
            },
        );

        let result = expand_inheritance(&mut templates);
        assert!(matches!(
            result,
            Err(TemplateLoadError::TemplateNotFound(_))
        ));
    }

    #[test]
    fn test_template_serialize_round_trip() {
        let tmpl = developer_template();
        let json = serde_json::to_string(&tmpl).unwrap();
        let restored: Template = serde_json::from_str(&json).unwrap();
        assert_eq!(restored.name, tmpl.name);
        assert_eq!(restored.actions.len(), tmpl.actions.len());
    }
}
