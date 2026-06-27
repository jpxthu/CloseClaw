//! Permission action helpers and builders
//!
//! Provides builder patterns and validation for [`Action`] types.

use crate::permission::engine::{Action, CommandArgs};

/// Builder for constructing [`Action`] variants fluently.
#[derive(Debug, Default)]
pub struct ActionBuilder {
    inner: Option<Action>,
}

impl ActionBuilder {
    /// Start building a File action.
    pub fn file(operation: impl Into<String>, paths: Vec<String>) -> Self {
        Self {
            inner: Some(Action::File {
                operation: operation.into(),
                paths,
            }),
        }
    }

    /// Start building a Command action.
    pub fn command(command: impl Into<String>) -> Self {
        Self {
            inner: Some(Action::Command {
                command: command.into(),
                args: CommandArgs::default(),
            }),
        }
    }

    /// Start building a Network action.
    pub fn network() -> Self {
        Self {
            inner: Some(Action::Network {
                hosts: Vec::new(),
                ports: Vec::new(),
            }),
        }
    }

    /// Start building a ToolCall action.
    pub fn tool_call(skill: impl Into<String>) -> Self {
        Self {
            inner: Some(Action::ToolCall {
                skill: skill.into(),
                methods: Vec::new(),
            }),
        }
    }

    /// Start building an InterAgent action.
    pub fn inter_agent() -> Self {
        Self {
            inner: Some(Action::InterAgent { agents: Vec::new() }),
        }
    }

    /// Start building a ConfigWrite action.
    pub fn config_write() -> Self {
        Self {
            inner: Some(Action::ConfigWrite { files: Vec::new() }),
        }
    }

    /// Finalize and return the constructed [`Action`].
    pub fn build(self) -> Option<Action> {
        self.inner
    }
}

impl ActionBuilder {
    /// Add allowed command arguments.
    pub fn allowed_args(mut self, args: Vec<String>) -> Self {
        if let Some(Action::Command { args: cmd_args, .. }) = &mut self.inner {
            *cmd_args = CommandArgs::Allowed { allowed: args };
        }
        self
    }

    /// Add blocked command arguments.
    pub fn blocked_args(mut self, args: Vec<String>) -> Self {
        if let Some(Action::Command { args: cmd_args, .. }) = &mut self.inner {
            *cmd_args = CommandArgs::Blocked { blocked: args };
        }
        self
    }

    /// Add hosts to a Network action.
    pub fn with_hosts(mut self, hosts: Vec<String>) -> Self {
        if let Some(Action::Network { hosts: h, .. }) = &mut self.inner {
            *h = hosts;
        }
        self
    }

    /// Add ports to a Network action.
    pub fn with_ports(mut self, ports: Vec<u16>) -> Self {
        if let Some(Action::Network { ports: p, .. }) = &mut self.inner {
            *p = ports;
        }
        self
    }

    /// Add methods to a ToolCall action.
    pub fn with_methods(mut self, methods: Vec<String>) -> Self {
        if let Some(Action::ToolCall { methods: m, .. }) = &mut self.inner {
            *m = methods;
        }
        self
    }

    /// Add agents to an InterAgent action.
    pub fn with_agents(mut self, agents: Vec<String>) -> Self {
        if let Some(Action::InterAgent { agents: a, .. }) = &mut self.inner {
            *a = agents;
        }
        self
    }

    /// Add files to a ConfigWrite action.
    pub fn with_files(mut self, files: Vec<String>) -> Self {
        if let Some(Action::ConfigWrite { files: f, .. }) = &mut self.inner {
            *f = files;
        }
        self
    }
}

impl From<ActionBuilder> for Option<Action> {
    fn from(builder: ActionBuilder) -> Self {
        builder.build()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_action_builder_file() {
        let action = ActionBuilder::file("read", vec!["/home/**".to_string()])
            .build()
            .unwrap();

        match action {
            Action::File { operation, paths } => {
                assert_eq!(operation, "read");
                assert_eq!(paths, vec!["/home/**".to_string()]);
            }
            _ => panic!("expected File action"),
        }
    }

    #[test]
    fn test_action_builder_command() {
        let action = ActionBuilder::command("cargo")
            .allowed_args(vec!["build".to_string(), "test".to_string()])
            .build()
            .unwrap();

        match action {
            Action::Command { command, args } => {
                assert_eq!(command, "cargo");
                assert!(matches!(args, CommandArgs::Allowed { .. }));
            }
            _ => panic!("expected Command action"),
        }
    }

    #[test]
    fn test_action_builder_network() {
        let action = ActionBuilder::network()
            .with_hosts(vec!["*.internal.corp".to_string()])
            .with_ports(vec![443, 8080])
            .build()
            .unwrap();

        match action {
            Action::Network { hosts, ports } => {
                assert_eq!(hosts.len(), 1);
                assert_eq!(ports, vec![443, 8080]);
            }
            _ => panic!("expected Network action"),
        }
    }
}
