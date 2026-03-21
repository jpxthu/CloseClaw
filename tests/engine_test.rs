use closeclaw::permission::{
    Effect, PermissionEngine, PermissionRequest, PermissionResponse,
    Rule, RuleSet,
};

fn test_rules_json() -> &'static str {
    r#"{
  "version": "1.0",
  "rules": [
    {
      "name": "dev-agent-file-read",
      "subject": { "agent": "dev-agent-01" },
      "effect": "allow",
      "actions": [
        {
          "type": "file",
          "operation": "read",
          "paths": ["/home/admin/code/**"]
        }
      ]
    },
    {
      "name": "dev-agent-file-write",
      "subject": { "agent": "dev-agent-01" },
      "effect": "allow",
      "actions": [
        {
          "type": "file",
          "operation": "write",
          "paths": ["/home/admin/code/closeclaw/src/**"]
        }
      ]
    },
    {
      "name": "dev-agent-git",
      "subject": { "agent": "dev-agent-01" },
      "effect": "allow",
      "actions": [
        {
          "type": "command",
          "command": "git",
          "args": { "allowed": ["status", "log", "diff", "add", "commit", "push", "pull"] }
        }
      ]
    },
    {
      "name": "dev-agent-forbidden-git-reset",
      "subject": { "agent": "dev-agent-01" },
      "effect": "deny",
      "actions": [
        {
          "type": "command",
          "command": "git",
          "args": { "blocked": ["reset", "rebase", "push", "--force"] }
        }
      ]
    },
    {
      "name": "readonly-agent",
      "subject": { "agent": "readonly-*", "match": "glob" },
      "effect": "allow",
      "actions": [
        { "type": "file", "operation": "read", "paths": ["**"] }
      ]
    }
  ],
  "defaults": {
    "file": "deny",
    "command": "deny",
    "network": "deny",
    "inter_agent": "deny",
    "config": "deny"
  }
}"#
}

#[tokio::test]
async fn test_rule_parsing() {
    let json = test_rules_json();
    let rules: RuleSet = serde_json::from_str(json).expect("Failed to parse rules");
    
    assert_eq!(rules.version, "1.0");
    assert_eq!(rules.rules.len(), 5);
    assert_eq!(rules.defaults.file, Effect::Deny);
}

#[tokio::test]
async fn test_file_read_allowed() {
    let json = test_rules_json();
    let rules: RuleSet = serde_json::from_str(json).expect("Failed to parse rules");
    
    let engine = PermissionEngine::new(rules);
    
    let request = PermissionRequest::FileOp {
        agent: "dev-agent-01".to_string(),
        path: "/home/admin/code/closeclaw/src/main.rs".to_string(),
        op: "read".to_string(),
    };
    
    let response = engine.evaluate(request).await;
    assert!(matches!(response, PermissionResponse::Allowed { .. }));
}

#[tokio::test]
async fn test_file_read_denied_no_match() {
    let json = test_rules_json();
    let rules: RuleSet = serde_json::from_str(json).expect("Failed to parse rules");
    
    let engine = PermissionEngine::new(rules);
    
    let request = PermissionRequest::FileOp {
        agent: "dev-agent-01".to_string(),
        path: "/etc/passwd".to_string(),
        op: "read".to_string(),
    };
    
    let response = engine.evaluate(request).await;
    assert!(matches!(response, PermissionResponse::Denied { .. }));
}

#[tokio::test]
async fn test_file_write_allowed() {
    let json = test_rules_json();
    let rules: RuleSet = serde_json::from_str(json).expect("Failed to parse rules");
    
    let engine = PermissionEngine::new(rules);
    
    let request = PermissionRequest::FileOp {
        agent: "dev-agent-01".to_string(),
        path: "/home/admin/code/closeclaw/src/main.rs".to_string(),
        op: "write".to_string(),
    };
    
    let response = engine.evaluate(request).await;
    assert!(matches!(response, PermissionResponse::Allowed { .. }));
}

#[tokio::test]
async fn test_command_allowed() {
    let json = test_rules_json();
    let rules: RuleSet = serde_json::from_str(json).expect("Failed to parse rules");
    
    let engine = PermissionEngine::new(rules);
    
    let request = PermissionRequest::CommandExec {
        agent: "dev-agent-01".to_string(),
        cmd: "git".to_string(),
        args: vec!["status".to_string()],
    };
    
    let response = engine.evaluate(request).await;
    assert!(matches!(response, PermissionResponse::Allowed { .. }));
}

#[tokio::test]
async fn test_command_denied_blocked() {
    let json = test_rules_json();
    let rules: RuleSet = serde_json::from_str(json).expect("Failed to parse rules");
    
    let engine = PermissionEngine::new(rules);
    
    let request = PermissionRequest::CommandExec {
        agent: "dev-agent-01".to_string(),
        cmd: "git".to_string(),
        args: vec!["reset".to_string(), "--hard".to_string()],
    };
    
    let response = engine.evaluate(request).await;
    assert!(matches!(response, PermissionResponse::Denied { .. }));
}

#[tokio::test]
async fn test_glob_matching() {
    let json = test_rules_json();
    let rules: RuleSet = serde_json::from_str(json).expect("Failed to parse rules");
    
    let engine = PermissionEngine::new(rules);
    
    let request = PermissionRequest::FileOp {
        agent: "readonly-agent-42".to_string(),
        path: "/any/path/in/the/system.txt".to_string(),
        op: "read".to_string(),
    };
    
    let response = engine.evaluate(request).await;
    assert!(matches!(response, PermissionResponse::Allowed { .. }));
}

#[tokio::test]
async fn test_default_deny() {
    let json = test_rules_json();
    let rules: RuleSet = serde_json::from_str(json).expect("Failed to parse rules");
    
    let engine = PermissionEngine::new(rules);
    
    let request = PermissionRequest::NetOp {
        agent: "dev-agent-01".to_string(),
        host: "example.com".to_string(),
        port: 443,
    };
    
    let response = engine.evaluate(request).await;
    assert!(matches!(response, PermissionResponse::Denied { .. }));
}

#[tokio::test]
async fn test_network_action_type() {
    let json = test_rules_json();
    let rules: RuleSet = serde_json::from_str(json).expect("Failed to parse rules");
    
    let engine = PermissionEngine::new(rules);
    
    let request = PermissionRequest::NetOp {
        agent: "dev-agent-01".to_string(),
        host: "api.github.com".to_string(),
        port: 443,
    };
    
    let response = engine.evaluate(request).await;
    assert!(matches!(response, PermissionResponse::Denied { .. }));
}

#[tokio::test]
async fn test_tool_call_action_type() {
    let json = test_rules_json();
    let rules: RuleSet = serde_json::from_str(json).expect("Failed to parse rules");
    
    let engine = PermissionEngine::new(rules);
    
    let request = PermissionRequest::ToolCall {
        agent: "dev-agent-01".to_string(),
        skill: "file_ops".to_string(),
        method: "read_file".to_string(),
    };
    
    let response = engine.evaluate(request).await;
    assert!(matches!(response, PermissionResponse::Denied { .. }));
}

#[tokio::test]
async fn test_inter_agent_action_type() {
    let json = test_rules_json();
    let rules: RuleSet = serde_json::from_str(json).expect("Failed to parse rules");
    
    let engine = PermissionEngine::new(rules);
    
    let request = PermissionRequest::InterAgentMsg {
        from: "agent-a".to_string(),
        to: "agent-b".to_string(),
    };
    
    let response = engine.evaluate(request).await;
    assert!(matches!(response, PermissionResponse::Denied { .. }));
}

#[tokio::test]
async fn test_config_write_action_type() {
    let json = test_rules_json();
    let rules: RuleSet = serde_json::from_str(json).expect("Failed to parse rules");
    
    let engine = PermissionEngine::new(rules);
    
    let request = PermissionRequest::ConfigWrite {
        agent: "dev-agent-01".to_string(),
        config_file: "agents.json".to_string(),
    };
    
    let response = engine.evaluate(request).await;
    assert!(matches!(response, PermissionResponse::Denied { .. }));
}

#[tokio::test]
async fn test_rule_subject_matching_exact() {
    let rule = Rule::parse_subject("dev-agent-01");
    assert!(rule.matches("dev-agent-01"));
    assert!(!rule.matches("dev-agent-02"));
}

#[tokio::test]
async fn test_rule_subject_matching_glob() {
    let rule = Rule::parse_subject_with_match("readonly-*", "glob");
    assert!(rule.matches("readonly-agent-1"));
    assert!(rule.matches("readonly-agent-42"));
    assert!(!rule.matches("readonly")); 
}

#[tokio::test]
async fn test_o1_lookup_performance() {
    let json = test_rules_json();
    let rules: RuleSet = serde_json::from_str(json).expect("Failed to parse rules");
    
    let engine = PermissionEngine::new(rules);
    
    let start = std::time::Instant::now();
    for _ in 0..1000 {
        let request = PermissionRequest::FileOp {
            agent: "dev-agent-01".to_string(),
            path: "/home/admin/code/closeclaw/src/main.rs".to_string(),
            op: "read".to_string(),
        };
        let _ = engine.evaluate(request);
    }
    let elapsed = start.elapsed();
    
    assert!(elapsed.as_millis() < 100, "O(1) lookup should be fast, took {:?}", elapsed);
}

#[tokio::test]
async fn test_unknown_agent_defaults_to_deny() {
    let json = test_rules_json();
    let rules: RuleSet = serde_json::from_str(json).expect("Failed to parse rules");
    
    let engine = PermissionEngine::new(rules);
    
    let request = PermissionRequest::FileOp {
        agent: "unknown-agent".to_string(),
        path: "/home/admin/code/**".to_string(),
        op: "read".to_string(),
    };
    
    
    let response = engine.evaluate(request).await;
    assert!(matches!(response, PermissionResponse::Denied { .. }));
}
