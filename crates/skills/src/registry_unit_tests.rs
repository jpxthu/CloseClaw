#[cfg(test)]
mod tests {
    use crate::registry::*;

    use crate::disk::types::SkillEffort;

    struct MockSkill {
        manifest: SkillManifest,
    }

    impl MockSkill {
        fn new(name: &str) -> Self {
            Self {
                manifest: SkillManifest {
                    name: name.to_string(),
                    description: format!("mock skill {}", name),
                    when_to_use: format!("use {} when needed", name),
                    context: crate::disk::types::SkillContext::default(),
                    effort: SkillEffort::Unknown,
                    paths: vec![],
                    user_invocable: false,
                },
            }
        }

        fn with_manifest(_name: &str, manifest: SkillManifest) -> Self {
            Self { manifest }
        }
    }

    #[async_trait]
    impl Skill for MockSkill {
        fn manifest(&self) -> SkillManifest {
            self.manifest.clone()
        }

        fn body(&self) -> &str {
            "mock body"
        }
    }

    #[tokio::test]
    async fn test_register_and_get() {
        let registry = BuiltinSkillRegistry::new();
        let skill = Arc::new(MockSkill::new("test_skill"));
        registry.register(skill).await;

        let found = registry.get("test_skill").await;
        assert!(found.is_some());
        assert_eq!(found.unwrap().manifest().name, "test_skill");
    }

    #[tokio::test]
    async fn test_get_not_found() {
        let registry = BuiltinSkillRegistry::new();
        let found = registry.get("nonexistent").await;
        assert!(found.is_none());
    }

    #[tokio::test]
    async fn test_list() {
        let registry = BuiltinSkillRegistry::new();
        registry.register(Arc::new(MockSkill::new("skill_a"))).await;
        registry.register(Arc::new(MockSkill::new("skill_b"))).await;

        let mut names = registry.list().await;
        names.sort();
        assert_eq!(names, vec!["skill_a", "skill_b"]);
    }

    #[tokio::test]
    async fn test_contains() {
        let registry = BuiltinSkillRegistry::new();
        registry.register(Arc::new(MockSkill::new("exists"))).await;

        assert!(registry.contains("exists").await);
        assert!(!registry.contains("missing").await);
    }

    #[tokio::test]
    async fn test_unregister() {
        let registry = BuiltinSkillRegistry::new();
        registry
            .register(Arc::new(MockSkill::new("to_remove")))
            .await;

        assert!(registry.unregister("to_remove").await);
        assert!(!registry.contains("to_remove").await);
        assert!(!registry.unregister("to_remove").await);
    }

    #[tokio::test]
    async fn test_register_replaces() {
        let registry = BuiltinSkillRegistry::new();
        registry.register(Arc::new(MockSkill::new("skill"))).await;
        registry.register(Arc::new(MockSkill::new("skill"))).await;

        let names = registry.list().await;
        assert_eq!(names.len(), 1);
    }

    #[tokio::test]
    async fn test_body_returns_value() {
        let registry = BuiltinSkillRegistry::new();
        registry
            .register(Arc::new(MockSkill::new("body_skill")))
            .await;

        let skill = registry.get("body_skill").await.unwrap();
        assert_eq!(skill.body(), "mock body");
    }

    #[tokio::test]
    async fn test_skill_error_display() {
        let err = SkillError::NotFound("test".to_string());
        assert!(err.to_string().contains("test"));

        let err = SkillError::ExecutionFailed("boom".to_string());
        assert!(err.to_string().contains("boom"));

        let err = SkillError::InvalidArgs("bad".to_string());
        assert!(err.to_string().contains("bad"));
    }

    #[test]
    fn test_skill_manifest_serialization() {
        let manifest = SkillManifest {
            name: "test".to_string(),
            description: "desc".to_string(),
            when_to_use: "use when testing".to_string(),
            context: crate::disk::types::SkillContext::default(),
            effort: SkillEffort::Small,
            paths: vec!["**/*.rs".to_string()],
            user_invocable: true,
        };
        let json = serde_json::to_string(&manifest).unwrap();
        let parsed: SkillManifest = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.name, "test");
        assert_eq!(parsed.when_to_use, "use when testing");
        assert!(parsed.user_invocable);
        assert_eq!(parsed.paths, vec!["**/*.rs".to_string()]);
    }

    #[test]
    fn test_registry_default() {
        let registry = BuiltinSkillRegistry::default();
        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(async {
            let names = registry.list().await;
            assert!(names.is_empty());
        });
    }

    #[test]
    fn test_default_manifest() {
        // A skill with no custom manifest fields has sensible defaults.
        struct NoManifestSkill;
        #[async_trait]
        impl Skill for NoManifestSkill {
            fn manifest(&self) -> SkillManifest {
                SkillManifest {
                    name: "no_manifest".into(),
                    description: "".into(),
                    when_to_use: String::new(),
                    context: crate::disk::types::SkillContext::default(),
                    effort: SkillEffort::Unknown,
                    paths: vec![],
                    user_invocable: false,
                }
            }
            fn body(&self) -> &str {
                ""
            }
        }

        let skill = NoManifestSkill;
        let m = skill.manifest();
        assert!(!m.user_invocable, "default should be user_invocable: false");
        assert!(m.when_to_use.is_empty());
        assert!(m.paths.is_empty());
        assert_eq!(m.effort, SkillEffort::Unknown);
    }

    #[test]
    fn test_mock_manifest() {
        let skill = MockSkill::new("test");
        let m = skill.manifest();
        assert_eq!(m.when_to_use, "use test when needed");
        assert!(!m.user_invocable);
        assert!(m.paths.is_empty());
        assert_eq!(m.effort, SkillEffort::Unknown);
    }

    #[test]
    fn test_mock_manifest_with_manifest() {
        let skill = MockSkill::with_manifest(
            "custom",
            SkillManifest {
                name: "custom".into(),
                description: "custom skill".into(),
                when_to_use: "custom when".into(),
                context: crate::disk::types::SkillContext::default(),
                effort: SkillEffort::Large,
                paths: vec!["**/*.rs".into()],
                user_invocable: true,
            },
        );
        let m = skill.manifest();
        assert_eq!(m.when_to_use, "custom when");
        assert!(m.user_invocable);
        assert_eq!(m.paths, vec!["**/*.rs"]);
        assert_eq!(m.effort, SkillEffort::Large);
    }

    #[tokio::test]
    async fn test_from_skills_registers_all() {
        let skills: Vec<Arc<dyn Skill>> = vec![
            Arc::new(MockSkill::new("alpha")),
            Arc::new(MockSkill::new("beta")),
        ];
        let registry = BuiltinSkillRegistry::from_skills(skills).await;
        let mut names = registry.list().await;
        names.sort();
        assert_eq!(names, vec!["alpha", "beta"]);
        assert!(registry.contains("alpha").await);
        assert!(registry.contains("beta").await);
    }

    #[tokio::test]
    async fn test_from_skills_empty() {
        let registry = BuiltinSkillRegistry::from_skills(vec![]).await;
        assert!(registry.list().await.is_empty());
    }

    #[tokio::test]
    async fn test_from_skills_overwrites_duplicates() {
        let skills: Vec<Arc<dyn Skill>> = vec![
            Arc::new(MockSkill::new("dup")),
            Arc::new(MockSkill::new("dup")),
        ];
        let registry = BuiltinSkillRegistry::from_skills(skills).await;
        let names = registry.list().await;
        assert_eq!(names.len(), 1);
        assert_eq!(names[0], "dup");
    }

    #[tokio::test]
    async fn test_generate_listing_only_user_invocable() {
        let registry = BuiltinSkillRegistry::from_skills(vec![
            Arc::new(MockSkill::with_manifest(
                "visible",
                SkillManifest {
                    name: "visible".into(),
                    description: "mock skill visible".into(),
                    when_to_use: "when visible".into(),
                    context: crate::disk::types::SkillContext::default(),
                    effort: SkillEffort::Small,
                    paths: vec![],
                    user_invocable: true,
                },
            )),
            Arc::new(MockSkill::with_manifest(
                "hidden",
                SkillManifest {
                    name: "hidden".into(),
                    description: "mock skill hidden".into(),
                    when_to_use: "when hidden".into(),
                    context: crate::disk::types::SkillContext::default(),
                    effort: SkillEffort::Unknown,
                    paths: vec![],
                    user_invocable: false,
                },
            )),
        ])
        .await;
        let listing = registry.generate_listing().await;
        assert!(listing.contains("visible"));
        assert!(!listing.contains("hidden"));
    }

    #[tokio::test]
    async fn test_generate_listing_format_matches_disk() {
        let registry = BuiltinSkillRegistry::from_skills(vec![Arc::new(MockSkill::with_manifest(
            "my_skill",
            SkillManifest {
                name: "my_skill".into(),
                description: "mock skill my_skill".into(),
                when_to_use: "use when testing".into(),
                context: crate::disk::types::SkillContext::default(),
                effort: SkillEffort::Medium,
                paths: vec![],
                user_invocable: true,
            },
        ))])
        .await;
        let listing = registry.generate_listing().await;
        assert_eq!(
            listing,
            "- **my_skill**: mock skill my_skill — use when testing [effort: medium]"
        );
    }

    #[tokio::test]
    async fn test_generate_listing_sorts_alphabetically() {
        let registry = BuiltinSkillRegistry::from_skills(vec![
            Arc::new(MockSkill::with_manifest(
                "zebra",
                SkillManifest {
                    name: "zebra".into(),
                    description: "mock skill zebra".into(),
                    when_to_use: String::new(),
                    context: crate::disk::types::SkillContext::default(),
                    effort: SkillEffort::Unknown,
                    paths: vec![],
                    user_invocable: true,
                },
            )),
            Arc::new(MockSkill::with_manifest(
                "alpha",
                SkillManifest {
                    name: "alpha".into(),
                    description: "mock skill alpha".into(),
                    when_to_use: String::new(),
                    context: crate::disk::types::SkillContext::default(),
                    effort: SkillEffort::Unknown,
                    paths: vec![],
                    user_invocable: true,
                },
            )),
        ])
        .await;
        let listing = registry.generate_listing().await;
        let alpha_pos = listing.find("alpha").unwrap();
        let zebra_pos = listing.find("zebra").unwrap();
        assert!(alpha_pos < zebra_pos);
    }

    #[tokio::test]
    async fn test_generate_listing_empty_when_no_invocable() {
        let registry =
            BuiltinSkillRegistry::from_skills(vec![Arc::new(MockSkill::new("hidden"))]).await;
        let listing = registry.generate_listing().await;
        assert!(listing.is_empty());
    }

    #[tokio::test]
    async fn test_generate_listing_excluding_conditional() {
        let registry = BuiltinSkillRegistry::from_skills(vec![
            Arc::new(MockSkill::with_manifest(
                "regular",
                SkillManifest {
                    name: "regular".into(),
                    description: "mock skill regular".into(),
                    when_to_use: "always".into(),
                    context: crate::disk::types::SkillContext::default(),
                    effort: SkillEffort::Unknown,
                    paths: vec![],
                    user_invocable: true,
                },
            )),
            Arc::new(MockSkill::with_manifest(
                "conditional",
                SkillManifest {
                    name: "conditional".into(),
                    description: "mock skill conditional".into(),
                    when_to_use: "on match".into(),
                    context: crate::disk::types::SkillContext::default(),
                    effort: SkillEffort::Small,
                    paths: vec!["**/*.rs".into()],
                    user_invocable: true,
                },
            )),
        ])
        .await;
        let listing = registry.generate_listing_excluding_conditional().await;
        assert!(listing.contains("regular"));
        assert!(!listing.contains("conditional"));
    }

    #[tokio::test]
    async fn test_find_conditional_matches() {
        let registry = BuiltinSkillRegistry::from_skills(vec![
            Arc::new(MockSkill::with_manifest(
                "rust_skill",
                SkillManifest {
                    name: "rust_skill".into(),
                    description: "mock skill rust_skill".into(),
                    when_to_use: "for rust files".into(),
                    context: crate::disk::types::SkillContext::default(),
                    effort: SkillEffort::Small,
                    paths: vec!["**/*.rs".into()],
                    user_invocable: true,
                },
            )),
            Arc::new(MockSkill::with_manifest(
                "no_paths",
                SkillManifest {
                    name: "no_paths".into(),
                    description: "mock skill no_paths".into(),
                    when_to_use: String::new(),
                    context: crate::disk::types::SkillContext::default(),
                    effort: SkillEffort::Unknown,
                    paths: vec![],
                    user_invocable: true,
                },
            )),
        ])
        .await;
        let matches = registry
            .find_conditional_matches(&[std::path::PathBuf::from("src/main.rs")])
            .await;
        assert_eq!(matches.len(), 1);
        assert_eq!(matches[0].name, "rust_skill");
        assert!(matches[0]
            .listing_line
            .contains("⚡ auto-activates on: **/*.rs"));
    }

    #[tokio::test]
    async fn test_find_conditional_matches_empty_paths() {
        let registry = BuiltinSkillRegistry::from_skills(vec![Arc::new(MockSkill::with_manifest(
            "skill",
            SkillManifest {
                name: "skill".into(),
                description: "mock skill skill".into(),
                when_to_use: String::new(),
                context: crate::disk::types::SkillContext::default(),
                effort: SkillEffort::Unknown,
                paths: vec!["**/*.rs".into()],
                user_invocable: true,
            },
        ))])
        .await;
        let matches = registry.find_conditional_matches(&[]).await;
        assert!(matches.is_empty());
    }

    #[tokio::test]
    async fn test_find_conditional_matches_no_match() {
        let registry = BuiltinSkillRegistry::from_skills(vec![Arc::new(MockSkill::with_manifest(
            "skill",
            SkillManifest {
                name: "skill".into(),
                description: "mock skill skill".into(),
                when_to_use: String::new(),
                context: crate::disk::types::SkillContext::default(),
                effort: SkillEffort::Unknown,
                paths: vec!["**/*.rs".into()],
                user_invocable: true,
            },
        ))])
        .await;
        let matches = registry
            .find_conditional_matches(&[std::path::PathBuf::from("file.txt")])
            .await;
        assert!(matches.is_empty());
    }

    #[tokio::test]
    async fn test_render_single_listing_no_when_to_use() {
        let manifest = SkillManifest {
            name: "bare".into(),
            description: "bare skill".into(),
            when_to_use: String::new(),
            context: crate::disk::types::SkillContext::default(),
            effort: SkillEffort::Unknown,
            paths: vec![],
            user_invocable: true,
        };
        let line = BuiltinSkillRegistry::render_single_listing(&manifest);
        assert_eq!(line, "- **bare**: bare skill");
    }

    #[tokio::test]
    async fn test_render_single_listing_with_paths() {
        let manifest = SkillManifest {
            name: "rs_skill".into(),
            description: "rust skill".into(),
            when_to_use: "for rust".into(),
            context: crate::disk::types::SkillContext::default(),
            effort: SkillEffort::Small,
            paths: vec!["**/*.rs".into(), "**/*.toml".into()],
            user_invocable: true,
        };
        let line = BuiltinSkillRegistry::render_single_listing(&manifest);
        assert_eq!(
            line,
            "- **rs_skill**: rust skill — for rust ⚡ auto-activates on: **/*.rs, **/*.toml [effort: small]"
        );
    }

    #[tokio::test]
    async fn test_user_invocable_names_empty() {
        let registry = BuiltinSkillRegistry::new();
        let names = registry.user_invocable_names().await;
        assert!(names.is_empty());
    }

    #[tokio::test]
    async fn test_user_invocable_names_only_invocable() {
        let registry = BuiltinSkillRegistry::from_skills(vec![Arc::new(MockSkill::with_manifest(
            "invocable",
            SkillManifest {
                name: "invocable".into(),
                description: "mock skill invocable".into(),
                when_to_use: String::new(),
                context: crate::disk::types::SkillContext::default(),
                effort: SkillEffort::Unknown,
                paths: vec![],
                user_invocable: true,
            },
        ))])
        .await;
        let names = registry.user_invocable_names().await;
        assert_eq!(names, vec!["invocable"]);
    }

    #[tokio::test]
    async fn test_user_invocable_names_mixed() {
        let registry = BuiltinSkillRegistry::from_skills(vec![
            Arc::new(MockSkill::with_manifest(
                "invocable_a",
                SkillManifest {
                    name: "invocable_a".into(),
                    description: "mock skill invocable_a".into(),
                    when_to_use: String::new(),
                    context: crate::disk::types::SkillContext::default(),
                    effort: SkillEffort::Unknown,
                    paths: vec![],
                    user_invocable: true,
                },
            )),
            Arc::new(MockSkill::with_manifest(
                "hidden_b",
                SkillManifest {
                    name: "hidden_b".into(),
                    description: "mock skill hidden_b".into(),
                    when_to_use: String::new(),
                    context: crate::disk::types::SkillContext::default(),
                    effort: SkillEffort::Unknown,
                    paths: vec![],
                    user_invocable: false,
                },
            )),
            Arc::new(MockSkill::with_manifest(
                "invocable_c",
                SkillManifest {
                    name: "invocable_c".into(),
                    description: "mock skill invocable_c".into(),
                    when_to_use: String::new(),
                    context: crate::disk::types::SkillContext::default(),
                    effort: SkillEffort::Unknown,
                    paths: vec![],
                    user_invocable: true,
                },
            )),
        ])
        .await;
        let mut names = registry.user_invocable_names().await;
        names.sort();
        assert_eq!(names, vec!["invocable_a", "invocable_c"]);
    }

    // ===================================================================
    // listing_entries_with_names
    // ===================================================================

    #[tokio::test]
    async fn test_builtin_listing_entries_no_whitelist() {
        let registry = BuiltinSkillRegistry::from_skills(vec![
            Arc::new(MockSkill::with_manifest(
                "alpha",
                SkillManifest {
                    name: "alpha".into(),
                    description: "mock skill alpha".into(),
                    when_to_use: String::new(),
                    context: crate::disk::types::SkillContext::default(),
                    effort: SkillEffort::Unknown,
                    paths: vec![],
                    user_invocable: true,
                },
            )),
            Arc::new(MockSkill::with_manifest(
                "beta",
                SkillManifest {
                    name: "beta".into(),
                    description: "mock skill beta".into(),
                    when_to_use: String::new(),
                    context: crate::disk::types::SkillContext::default(),
                    effort: SkillEffort::Unknown,
                    paths: vec![],
                    user_invocable: true,
                },
            )),
            Arc::new(MockSkill::with_manifest(
                "hidden",
                SkillManifest {
                    name: "hidden".into(),
                    description: "mock skill hidden".into(),
                    when_to_use: String::new(),
                    context: crate::disk::types::SkillContext::default(),
                    effort: SkillEffort::Unknown,
                    paths: vec![],
                    user_invocable: false,
                },
            )),
        ])
        .await;
        let entries = registry.listing_entries_with_names(None, false, None).await;
        let names: Vec<&str> = entries.iter().map(|(n, _, _)| n.as_str()).collect();
        assert!(names.contains(&"alpha"));
        assert!(names.contains(&"beta"));
        assert!(!names.contains(&"hidden"));
    }

    #[tokio::test]
    async fn test_builtin_listing_entries_specific_whitelist() {
        let registry = BuiltinSkillRegistry::from_skills(vec![
            Arc::new(MockSkill::with_manifest(
                "alpha",
                SkillManifest {
                    name: "alpha".into(),
                    description: "mock skill alpha".into(),
                    when_to_use: String::new(),
                    context: crate::disk::types::SkillContext::default(),
                    effort: SkillEffort::Unknown,
                    paths: vec![],
                    user_invocable: true,
                },
            )),
            Arc::new(MockSkill::with_manifest(
                "beta",
                SkillManifest {
                    name: "beta".into(),
                    description: "mock skill beta".into(),
                    when_to_use: String::new(),
                    context: crate::disk::types::SkillContext::default(),
                    effort: SkillEffort::Unknown,
                    paths: vec![],
                    user_invocable: true,
                },
            )),
        ])
        .await;
        let entries = registry
            .listing_entries_with_names(Some(&["beta".to_string()]), false, None)
            .await;
        let names: Vec<&str> = entries.iter().map(|(n, _, _)| n.as_str()).collect();
        assert!(!names.contains(&"alpha"));
        assert!(names.contains(&"beta"));
    }

    #[tokio::test]
    async fn test_builtin_listing_entries_wildcard_whitelist() {
        let registry = BuiltinSkillRegistry::from_skills(vec![
            Arc::new(MockSkill::with_manifest(
                "alpha",
                SkillManifest {
                    name: "alpha".into(),
                    description: "mock skill alpha".into(),
                    when_to_use: String::new(),
                    context: crate::disk::types::SkillContext::default(),
                    effort: SkillEffort::Unknown,
                    paths: vec![],
                    user_invocable: true,
                },
            )),
            Arc::new(MockSkill::with_manifest(
                "beta",
                SkillManifest {
                    name: "beta".into(),
                    description: "mock skill beta".into(),
                    when_to_use: String::new(),
                    context: crate::disk::types::SkillContext::default(),
                    effort: SkillEffort::Unknown,
                    paths: vec![],
                    user_invocable: true,
                },
            )),
        ])
        .await;
        let entries = registry
            .listing_entries_with_names(Some(&["*".to_string()]), false, None)
            .await;
        assert_eq!(entries.len(), 2);
    }

    #[tokio::test]
    async fn test_builtin_listing_entries_exclude_conditional() {
        let registry = BuiltinSkillRegistry::from_skills(vec![
            Arc::new(MockSkill::with_manifest(
                "plain",
                SkillManifest {
                    name: "plain".into(),
                    description: "mock skill plain".into(),
                    when_to_use: String::new(),
                    context: crate::disk::types::SkillContext::default(),
                    effort: SkillEffort::Unknown,
                    paths: vec![],
                    user_invocable: true,
                },
            )),
            Arc::new(MockSkill::with_manifest(
                "cond",
                SkillManifest {
                    name: "cond".into(),
                    description: "mock skill cond".into(),
                    when_to_use: String::new(),
                    context: crate::disk::types::SkillContext::default(),
                    effort: SkillEffort::Unknown,
                    paths: vec!["**/*.rs".into()],
                    user_invocable: true,
                },
            )),
        ])
        .await;
        let entries = registry.listing_entries_with_names(None, true, None).await;
        let names: Vec<&str> = entries.iter().map(|(n, _, _)| n.as_str()).collect();
        assert!(names.contains(&"plain"));
        assert!(!names.contains(&"cond"));
    }

    #[tokio::test]
    async fn test_builtin_listing_entries_activated_exempts_conditional() {
        let registry = BuiltinSkillRegistry::from_skills(vec![
            Arc::new(MockSkill::with_manifest(
                "plain",
                SkillManifest {
                    name: "plain".into(),
                    description: "mock skill plain".into(),
                    when_to_use: String::new(),
                    context: crate::disk::types::SkillContext::default(),
                    effort: SkillEffort::Unknown,
                    paths: vec![],
                    user_invocable: true,
                },
            )),
            Arc::new(MockSkill::with_manifest(
                "cond",
                SkillManifest {
                    name: "cond".into(),
                    description: "mock skill cond".into(),
                    when_to_use: String::new(),
                    context: crate::disk::types::SkillContext::default(),
                    effort: SkillEffort::Unknown,
                    paths: vec!["**/*.rs".into()],
                    user_invocable: true,
                },
            )),
        ])
        .await;
        let entries = registry
            .listing_entries_with_names(None, true, Some(&["cond".to_string()]))
            .await;
        let names: Vec<&str> = entries.iter().map(|(n, _, _)| n.as_str()).collect();
        assert!(names.contains(&"plain"));
        assert!(names.contains(&"cond"));
    }

    #[tokio::test]
    async fn test_builtin_listing_entries_sorted_by_source_then_name() {
        // All builtin skills have SkillSource::Bundled, so sorting is by name only.
        let registry = BuiltinSkillRegistry::from_skills(vec![
            Arc::new(MockSkill::with_manifest(
                "zebra",
                SkillManifest {
                    name: "zebra".into(),
                    description: "mock skill zebra".into(),
                    when_to_use: String::new(),
                    context: crate::disk::types::SkillContext::default(),
                    effort: SkillEffort::Unknown,
                    paths: vec![],
                    user_invocable: true,
                },
            )),
            Arc::new(MockSkill::with_manifest(
                "alpha",
                SkillManifest {
                    name: "alpha".into(),
                    description: "mock skill alpha".into(),
                    when_to_use: String::new(),
                    context: crate::disk::types::SkillContext::default(),
                    effort: SkillEffort::Unknown,
                    paths: vec![],
                    user_invocable: true,
                },
            )),
        ])
        .await;
        let entries = registry.listing_entries_with_names(None, false, None).await;
        assert_eq!(entries[0].0, "alpha");
        assert_eq!(entries[1].0, "zebra");
    }

    #[tokio::test]
    async fn test_builtin_listing_entries_line_contains_description() {
        let registry = BuiltinSkillRegistry::from_skills(vec![Arc::new(MockSkill::with_manifest(
            "my_skill",
            SkillManifest {
                name: "my_skill".into(),
                description: "mock skill my_skill".into(),
                when_to_use: String::new(),
                context: crate::disk::types::SkillContext::default(),
                effort: SkillEffort::Unknown,
                paths: vec![],
                user_invocable: true,
            },
        ))])
        .await;
        let entries = registry.listing_entries_with_names(None, false, None).await;
        assert_eq!(entries.len(), 1);
        assert!(entries[0].2.contains("**my_skill**"));
        assert!(entries[0].2.contains("mock skill my_skill"));
    }

    #[tokio::test]
    async fn test_builtin_listing_entries_all_builtin_source() {
        let registry = BuiltinSkillRegistry::from_skills(vec![Arc::new(MockSkill::with_manifest(
            "skill",
            SkillManifest {
                name: "skill".into(),
                description: "mock skill skill".into(),
                when_to_use: String::new(),
                context: crate::disk::types::SkillContext::default(),
                effort: SkillEffort::Unknown,
                paths: vec![],
                user_invocable: true,
            },
        ))])
        .await;
        let entries = registry.listing_entries_with_names(None, false, None).await;
        assert_eq!(entries[0].1, SkillSource::Bundled);
    }
}
