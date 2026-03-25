use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct McpServerConfig {
    pub name: String,
    pub command: String,
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
        struct ServerList { servers: Vec<McpServerConfig> }
        let list: ServerList = toml::from_str(toml_str).unwrap();
        assert_eq!(list.servers.len(), 2);
        assert_eq!(list.servers[0].name, "filesystem");
        assert_eq!(list.servers[1].name, "github");
    }

    #[test]
    fn empty_servers_deserializes_to_empty_vec() {
        #[derive(serde::Deserialize)]
        struct ServerList { #[serde(default)] servers: Vec<McpServerConfig> }
        let list: ServerList = toml::from_str("").unwrap();
        assert_eq!(list.servers.len(), 0);
    }
}
