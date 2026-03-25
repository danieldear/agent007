// crates/zones/src/config.rs
use crate::level::ZoneLevel;

/// Zone rules loaded from config.toml or constructed programmatically.
#[derive(Debug, Clone, Default, serde::Deserialize, serde::Serialize)]
pub struct ZoneConfig {
    #[serde(default)]
    pub forbidden:    Vec<String>,
    #[serde(default)]
    pub readonly:     Vec<String>,
    #[serde(default)]
    pub sensitive:    Vec<String>,
    #[serde(default)]
    pub unrestricted: Vec<String>,
}

impl ZoneConfig {
    /// Return the pattern list for a given ZoneLevel.
    pub fn patterns_for(&self, level: ZoneLevel) -> &[String] {
        match level {
            ZoneLevel::Forbidden    => &self.forbidden,
            ZoneLevel::Readonly     => &self.readonly,
            ZoneLevel::Sensitive    => &self.sensitive,
            ZoneLevel::Unrestricted => &self.unrestricted,
        }
    }
}

/// Top-level wrapper matching the [zones] TOML section.
#[derive(Debug, Clone, Default, serde::Deserialize, serde::Serialize)]
pub struct ZonesTomlWrapper {
    #[serde(default)]
    pub zones: ZoneConfig,
}

impl ZoneConfig {
    /// Parse a ZoneConfig from a TOML string containing a `[zones]` section.
    pub fn from_toml(s: &str) -> Result<Self, toml::de::Error> {
        let wrapper: ZonesTomlWrapper = toml::from_str(s)?;
        Ok(wrapper.zones)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE_ZONES_TOML: &str = r#"
[zones]
forbidden    = ["secrets/", "keys/", ".env", "*.pem"]
readonly     = ["src/auth/", "src/payment/"]
sensitive    = ["src/crypto/"]
unrestricted = ["src/", "tests/", "docs/"]
"#;

    #[test]
    fn parse_zones_toml_forbidden() {
        let config = ZoneConfig::from_toml(SAMPLE_ZONES_TOML).unwrap();
        assert_eq!(config.forbidden, vec!["secrets/", "keys/", ".env", "*.pem"]);
    }

    #[test]
    fn parse_zones_toml_readonly() {
        let config = ZoneConfig::from_toml(SAMPLE_ZONES_TOML).unwrap();
        assert_eq!(config.readonly, vec!["src/auth/", "src/payment/"]);
    }

    #[test]
    fn parse_zones_toml_sensitive() {
        let config = ZoneConfig::from_toml(SAMPLE_ZONES_TOML).unwrap();
        assert_eq!(config.sensitive, vec!["src/crypto/"]);
    }

    #[test]
    fn parse_zones_toml_unrestricted() {
        let config = ZoneConfig::from_toml(SAMPLE_ZONES_TOML).unwrap();
        assert_eq!(config.unrestricted, vec!["src/", "tests/", "docs/"]);
    }

    #[test]
    fn parse_empty_toml_gives_empty_config() {
        let config = ZoneConfig::from_toml("").unwrap();
        assert!(config.forbidden.is_empty());
        assert!(config.readonly.is_empty());
    }

    #[test]
    fn parse_partial_zones_toml_uses_defaults() {
        let toml_str = "[zones]\nforbidden = [\".env\"]\n";
        let config = ZoneConfig::from_toml(toml_str).unwrap();
        assert_eq!(config.forbidden, vec![".env"]);
        assert!(config.readonly.is_empty());
        assert!(config.sensitive.is_empty());
    }

    #[test]
    fn patterns_for_returns_correct_slice() {
        let config = ZoneConfig {
            forbidden:    vec!["a".to_string()],
            readonly:     vec!["b".to_string()],
            sensitive:    vec!["c".to_string()],
            unrestricted: vec!["d".to_string()],
        };
        use crate::level::ZoneLevel;
        assert_eq!(config.patterns_for(ZoneLevel::Forbidden),    &["a"]);
        assert_eq!(config.patterns_for(ZoneLevel::Readonly),     &["b"]);
        assert_eq!(config.patterns_for(ZoneLevel::Sensitive),    &["c"]);
        assert_eq!(config.patterns_for(ZoneLevel::Unrestricted), &["d"]);
    }
}
