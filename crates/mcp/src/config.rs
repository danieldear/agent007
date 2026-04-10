use std::collections::HashMap;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct McpServerConfig {
    pub name: String,
    pub command: String,
    #[serde(default)]
    pub args: Vec<String>,
    #[serde(default)]
    pub env: HashMap<String, String>,
    pub cwd: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mcp_server_config_deserializes_from_toml() {
        let toml_str = r#"
[[servers]]
name = "filesystem"
command = "npx @modelcontextprotocol/server-filesystem /tmp"

[[servers]]
name = "github"
command = "npx @modelcontextprotocol/server-github"
"#;
        #[derive(serde::Deserialize)]
        struct ServerList {
            servers: Vec<McpServerConfig>,
        }
        let list: ServerList = toml::from_str(toml_str).unwrap();
        assert_eq!(list.servers.len(), 2);
        assert_eq!(list.servers[0].name, "filesystem");
        assert_eq!(list.servers[1].name, "github");
        assert!(list.servers[0].args.is_empty());
        assert!(list.servers[0].env.is_empty());
        assert!(list.servers[0].cwd.is_none());
    }

    #[test]
    fn empty_servers_deserializes_to_empty_vec() {
        #[derive(serde::Deserialize)]
        struct ServerList {
            #[serde(default)]
            servers: Vec<McpServerConfig>,
        }
        let list: ServerList = toml::from_str("").unwrap();
        assert_eq!(list.servers.len(), 0);
    }

    #[test]
    fn structured_server_deserializes() {
        let toml_str = r#"
[[servers]]
name = "filesystem"
command = "npx"
args = ["-y", "@modelcontextprotocol/server-filesystem", "/tmp"]
cwd = "/tmp"

[servers.env]
NODE_ENV = "production"
"#;
        #[derive(serde::Deserialize)]
        struct ServerList {
            servers: Vec<McpServerConfig>,
        }
        let list: ServerList = toml::from_str(toml_str).unwrap();
        let server = &list.servers[0];
        assert_eq!(server.command, "npx");
        assert_eq!(server.args[1], "@modelcontextprotocol/server-filesystem");
        assert_eq!(server.cwd.as_deref(), Some("/tmp"));
        assert_eq!(
            server.env.get("NODE_ENV").map(String::as_str),
            Some("production")
        );
    }
}
