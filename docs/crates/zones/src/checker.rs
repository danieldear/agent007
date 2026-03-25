// crates/zones/src/checker.rs
use std::path::Path;
use globset::{Glob, GlobSet, GlobSetBuilder};

use crate::config::ZoneConfig;
use crate::error::ZonesError;
use crate::level::{FileOp, ZoneLevel};

#[derive(Debug)]
pub struct ZoneViolation {
    pub path: std::path::PathBuf,
    pub zone: ZoneLevel,
    pub op: FileOp,
}

impl std::fmt::Display for ZoneViolation {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "zone violation: {} on '{}' (zone: {})",
            self.op.as_str(),
            self.path.display(),
            self.zone.as_str()
        )
    }
}

/// A compiled set of zone rules.
/// Rules are evaluated in priority order; the most restrictive matching level wins.
/// Unmatched paths default to `ZoneLevel::Unrestricted`.
pub struct ZoneChecker {
    rules: Vec<(GlobSet, ZoneLevel)>,
}

impl std::fmt::Debug for ZoneChecker {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ZoneChecker")
            .field("rules_count", &self.rules.len())
            .finish()
    }
}

impl ZoneChecker {
    /// Build a ZoneChecker from a single ZoneConfig.
    pub fn new(config: &ZoneConfig) -> Result<Self, ZonesError> {
        Self::from_configs(std::slice::from_ref(config))
    }

    /// Build a ZoneChecker from multiple ZoneConfigs (sub-orchestrator merging).
    /// Most restrictive level wins across all configs.
    pub fn from_configs(configs: &[ZoneConfig]) -> Result<Self, ZonesError> {
        let all_levels = [
            ZoneLevel::Forbidden,
            ZoneLevel::Sensitive,
            ZoneLevel::Readonly,
            ZoneLevel::Unrestricted,
        ];

        let mut rules: Vec<(GlobSet, ZoneLevel)> = Vec::new();

        for level in all_levels {
            let mut builder = GlobSetBuilder::new();
            let mut any = false;

            for config in configs {
                for pattern in config.patterns_for(level) {
                    // If the pattern ends with '/', automatically add '**' so that
                    // "secrets/" matches "secrets/token", "secrets/dir/file", etc.
                    let expanded = if pattern.ends_with('/') {
                        format!("{}**", pattern)
                    } else {
                        pattern.clone()
                    };
                    let glob = Glob::new(&expanded).map_err(|e| ZonesError::InvalidGlob {
                        pattern: pattern.to_string(),
                        source: e,
                    })?;
                    builder.add(glob);
                    any = true;
                }
            }

            if any {
                let set = builder.build().map_err(|e| ZonesError::InvalidGlob {
                    pattern: "<build>".to_string(),
                    source: e,
                })?;
                rules.push((set, level));
            }
        }

        Ok(Self { rules })
    }

    /// Return the most restrictive ZoneLevel matching `path`.
    /// If no pattern matches, returns `ZoneLevel::Unrestricted`.
    pub fn zone_for(&self, path: &Path) -> ZoneLevel {
        let path_str = path.to_string_lossy();
        let mut result = ZoneLevel::Unrestricted;

        for (glob_set, level) in &self.rules {
            if glob_set.is_match(path_str.as_ref()) {
                result = result.most_restrictive(*level);
            }
        }

        result
    }

    /// Check whether `op` is permitted on `path` given the current zone rules.
    /// Returns `Ok(())` if allowed, or `Err(ZoneViolation)` if blocked.
    pub fn check(&self, path: &Path, op: FileOp) -> Result<(), ZoneViolation> {
        let zone = self.zone_for(path);
        let allowed = match (zone, op) {
            // Unrestricted: all ops allowed
            (ZoneLevel::Unrestricted, _) => true,
            // Readonly: read allowed, write/delete blocked
            (ZoneLevel::Readonly, FileOp::Read)   => true,
            (ZoneLevel::Readonly, _)               => false,
            // Sensitive: read allowed, write/delete blocked
            (ZoneLevel::Sensitive, FileOp::Read)  => true,
            (ZoneLevel::Sensitive, _)              => false,
            // Forbidden: no ops allowed
            (ZoneLevel::Forbidden, _)              => false,
        };

        if allowed {
            Ok(())
        } else {
            Err(ZoneViolation {
                path: path.to_path_buf(),
                zone,
                op,
            })
        }
    }

    /// Returns true if the path's content may be stored in RAG / memory.
    /// False for `Sensitive` and `Forbidden`.
    pub fn should_index(&self, path: &Path) -> bool {
        match self.zone_for(path) {
            ZoneLevel::Unrestricted | ZoneLevel::Readonly => true,
            ZoneLevel::Sensitive | ZoneLevel::Forbidden   => false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::ZoneConfig;

    fn make_checker() -> ZoneChecker {
        let config = ZoneConfig {
            forbidden:    vec!["secrets/".to_string(), "keys/".to_string(), ".env".to_string(), "*.pem".to_string()],
            readonly:     vec!["src/auth/".to_string(), "src/payment/".to_string()],
            sensitive:    vec!["src/crypto/".to_string()],
            unrestricted: vec!["src/".to_string(), "tests/".to_string(), "docs/".to_string()],
        };
        ZoneChecker::new(&config).expect("ZoneChecker::new should not fail with valid globs")
    }

    // --- zone_for ---

    #[test]
    fn zone_for_forbidden_path_returns_forbidden() {
        let checker = make_checker();
        assert_eq!(checker.zone_for(Path::new("secrets/db_password")), ZoneLevel::Forbidden);
    }

    #[test]
    fn zone_for_pem_file_returns_forbidden() {
        let checker = make_checker();
        assert_eq!(checker.zone_for(Path::new("server.pem")), ZoneLevel::Forbidden);
    }

    #[test]
    fn zone_for_env_file_returns_forbidden() {
        let checker = make_checker();
        assert_eq!(checker.zone_for(Path::new(".env")), ZoneLevel::Forbidden);
    }

    #[test]
    fn zone_for_readonly_path_returns_readonly() {
        let checker = make_checker();
        assert_eq!(checker.zone_for(Path::new("src/auth/login.rs")), ZoneLevel::Readonly);
    }

    #[test]
    fn zone_for_sensitive_path_returns_sensitive() {
        let checker = make_checker();
        assert_eq!(checker.zone_for(Path::new("src/crypto/hash.rs")), ZoneLevel::Sensitive);
    }

    #[test]
    fn zone_for_unrestricted_path_returns_unrestricted() {
        let checker = make_checker();
        // "src/" is unrestricted BUT "src/auth/" is readonly — auth wins (most restrictive).
        // A plain src path not under auth/payment/crypto should be unrestricted.
        assert_eq!(checker.zone_for(Path::new("src/utils.rs")), ZoneLevel::Unrestricted);
    }

    #[test]
    fn zone_for_unmatched_path_defaults_to_unrestricted() {
        let checker = make_checker();
        assert_eq!(checker.zone_for(Path::new("README.md")), ZoneLevel::Unrestricted);
    }

    // --- check ---

    #[test]
    fn check_read_on_forbidden_is_err() {
        let checker = make_checker();
        assert!(checker.check(Path::new("secrets/token"), FileOp::Read).is_err());
    }

    #[test]
    fn check_write_on_forbidden_is_err() {
        let checker = make_checker();
        assert!(checker.check(Path::new("keys/private.key"), FileOp::Write).is_err());
    }

    #[test]
    fn check_read_on_readonly_is_ok() {
        let checker = make_checker();
        assert!(checker.check(Path::new("src/auth/login.rs"), FileOp::Read).is_ok());
    }

    #[test]
    fn check_write_on_readonly_is_err() {
        let checker = make_checker();
        let result = checker.check(Path::new("src/auth/login.rs"), FileOp::Write);
        assert!(result.is_err());
        let violation = result.unwrap_err();
        assert_eq!(violation.zone, ZoneLevel::Readonly);
        assert_eq!(violation.op, FileOp::Write);
    }

    #[test]
    fn check_read_on_sensitive_is_ok() {
        let checker = make_checker();
        assert!(checker.check(Path::new("src/crypto/hash.rs"), FileOp::Read).is_ok());
    }

    #[test]
    fn check_write_on_sensitive_is_err() {
        let checker = make_checker();
        assert!(checker.check(Path::new("src/crypto/hash.rs"), FileOp::Write).is_err());
    }

    #[test]
    fn check_write_on_unrestricted_is_ok() {
        let checker = make_checker();
        assert!(checker.check(Path::new("src/utils.rs"), FileOp::Write).is_ok());
    }

    // --- should_index ---

    #[test]
    fn should_index_true_for_unrestricted_and_readonly() {
        let checker = make_checker();
        assert!(checker.should_index(Path::new("src/utils.rs")));
        assert!(checker.should_index(Path::new("src/auth/login.rs")));
    }

    #[test]
    fn should_index_false_for_sensitive_and_forbidden() {
        let checker = make_checker();
        assert!(!checker.should_index(Path::new("src/crypto/hash.rs")));
        assert!(!checker.should_index(Path::new("secrets/token")));
    }

    // --- from_configs (multi-config merging) ---

    #[test]
    fn from_configs_most_restrictive_wins() {
        let config_a = ZoneConfig {
            forbidden:    vec![],
            readonly:     vec!["src/auth/".to_string()],
            sensitive:    vec![],
            unrestricted: vec!["src/".to_string()],
        };
        let config_b = ZoneConfig {
            forbidden:    vec!["src/auth/".to_string()],  // escalate to forbidden
            readonly:     vec![],
            sensitive:    vec![],
            unrestricted: vec![],
        };
        let checker = ZoneChecker::from_configs(&[config_a, config_b])
            .expect("from_configs should not fail");
        // Most restrictive (forbidden) wins
        assert_eq!(checker.zone_for(Path::new("src/auth/login.rs")), ZoneLevel::Forbidden);
    }

    #[test]
    fn invalid_glob_returns_error() {
        let config = ZoneConfig {
            forbidden:    vec!["[invalid".to_string()],
            readonly:     vec![],
            sensitive:    vec![],
            unrestricted: vec![],
        };
        let result = ZoneChecker::new(&config);
        assert!(result.is_err());
        assert!(matches!(result.unwrap_err(), crate::error::ZonesError::InvalidGlob { .. }));
    }
}
