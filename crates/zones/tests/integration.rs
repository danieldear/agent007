// crates/zones/tests/integration.rs
//! End-to-end integration tests for the zones crate.
//! These tests run as a separate test binary (Rust integration test convention).

use std::path::Path;
use tempfile::TempDir;

use agent007_zones::{AuditEntry, AuditLogger, FileOp, ZoneChecker, ZoneConfig, ZoneLevel};

fn make_config() -> ZoneConfig {
    ZoneConfig {
        forbidden: vec![
            "secrets/".to_string(),
            "keys/".to_string(),
            ".env".to_string(),
            "*.pem".to_string(),
        ],
        readonly: vec!["src/auth/".to_string(), "src/payment/".to_string()],
        sensitive: vec!["src/crypto/".to_string()],
        unrestricted: vec![
            "src/".to_string(),
            "tests/".to_string(),
            "docs/".to_string(),
        ],
    }
}

// --- Forbidden path ---

#[test]
fn forbidden_path_read_is_blocked() {
    let checker = ZoneChecker::new(&make_config()).unwrap();
    let result = checker.check(Path::new("secrets/db_password"), FileOp::Read);
    assert!(result.is_err(), "read on forbidden path must be blocked");
    let violation = result.unwrap_err();
    assert_eq!(violation.zone, ZoneLevel::Forbidden);
    assert_eq!(violation.op, FileOp::Read);
}

#[test]
fn forbidden_path_write_is_blocked() {
    let checker = ZoneChecker::new(&make_config()).unwrap();
    assert!(checker
        .check(Path::new("keys/private.pem"), FileOp::Write)
        .is_err());
}

#[test]
fn forbidden_pem_glob_matches() {
    let checker = ZoneChecker::new(&make_config()).unwrap();
    assert_eq!(
        checker.zone_for(Path::new("cert.pem")),
        ZoneLevel::Forbidden
    );
}

#[test]
fn forbidden_path_not_indexed() {
    let checker = ZoneChecker::new(&make_config()).unwrap();
    assert!(!checker.should_index(Path::new("secrets/token")));
}

// --- Readonly path ---

#[test]
fn readonly_path_read_is_allowed() {
    let checker = ZoneChecker::new(&make_config()).unwrap();
    assert!(checker
        .check(Path::new("src/auth/login.rs"), FileOp::Read)
        .is_ok());
}

#[test]
fn readonly_path_write_is_blocked() {
    let checker = ZoneChecker::new(&make_config()).unwrap();
    let result = checker.check(Path::new("src/auth/login.rs"), FileOp::Write);
    assert!(result.is_err());
    assert_eq!(result.unwrap_err().zone, ZoneLevel::Readonly);
}

#[test]
fn readonly_path_delete_is_blocked() {
    let checker = ZoneChecker::new(&make_config()).unwrap();
    assert!(checker
        .check(Path::new("src/payment/checkout.rs"), FileOp::Delete)
        .is_err());
}

#[test]
fn readonly_path_is_indexed() {
    let checker = ZoneChecker::new(&make_config()).unwrap();
    // Readonly paths CAN be indexed (content is safe to read/store)
    assert!(checker.should_index(Path::new("src/auth/login.rs")));
}

// --- Sensitive path ---

#[test]
fn sensitive_path_read_is_allowed() {
    let checker = ZoneChecker::new(&make_config()).unwrap();
    assert!(checker
        .check(Path::new("src/crypto/hash.rs"), FileOp::Read)
        .is_ok());
}

#[test]
fn sensitive_path_write_is_blocked() {
    let checker = ZoneChecker::new(&make_config()).unwrap();
    assert!(checker
        .check(Path::new("src/crypto/hash.rs"), FileOp::Write)
        .is_err());
}

#[test]
fn sensitive_path_not_indexed() {
    let checker = ZoneChecker::new(&make_config()).unwrap();
    assert!(!checker.should_index(Path::new("src/crypto/hash.rs")));
}

// --- Unrestricted path ---

#[test]
fn unrestricted_path_all_ops_allowed() {
    let checker = ZoneChecker::new(&make_config()).unwrap();
    let path = Path::new("src/utils.rs");
    assert!(checker.check(path, FileOp::Read).is_ok());
    assert!(checker.check(path, FileOp::Write).is_ok());
    assert!(checker.check(path, FileOp::Delete).is_ok());
}

#[test]
fn unrestricted_path_is_indexed() {
    let checker = ZoneChecker::new(&make_config()).unwrap();
    assert!(checker.should_index(Path::new("src/utils.rs")));
}

// --- Audit log integration ---

#[test]
fn audit_log_records_blocked_forbidden_read() {
    let dir = TempDir::new().unwrap();
    let log_path = dir.path().join("audit.log");
    let logger = AuditLogger::new(&log_path);

    let checker = ZoneChecker::new(&make_config()).unwrap();
    let path = Path::new("secrets/api_key");
    let result = checker.check(path, FileOp::Read);
    assert!(result.is_err());

    let violation = result.unwrap_err();
    let entry = AuditEntry::now(
        "IntegrationAgent",
        violation.op.as_str(),
        path.to_string_lossy().as_ref(),
        violation.zone.as_str(),
        false,
    );
    logger.log(&entry).unwrap();

    let lines = logger.read_lines().unwrap();
    assert_eq!(lines.len(), 1);
    let v: serde_json::Value = serde_json::from_str(&lines[0]).unwrap();
    assert_eq!(v["agent"], "IntegrationAgent");
    assert_eq!(v["action"], "read");
    assert_eq!(v["zone"], "forbidden");
    assert_eq!(v["allowed"], false);
    assert_eq!(v["blocked"], true);
}

#[test]
fn audit_log_records_allowed_readonly_read() {
    let dir = TempDir::new().unwrap();
    let log_path = dir.path().join("audit.log");
    let logger = AuditLogger::new(&log_path);

    let checker = ZoneChecker::new(&make_config()).unwrap();
    let path = Path::new("src/auth/login.rs");
    assert!(checker.check(path, FileOp::Read).is_ok());

    let zone = checker.zone_for(path);
    let entry = AuditEntry::now(
        "IntegrationAgent",
        "read",
        path.to_string_lossy().as_ref(),
        zone.as_str(),
        true,
    );
    logger.log(&entry).unwrap();

    let lines = logger.read_lines().unwrap();
    assert_eq!(lines.len(), 1);
    let v: serde_json::Value = serde_json::from_str(&lines[0]).unwrap();
    assert_eq!(v["zone"], "readonly");
    assert_eq!(v["allowed"], true);
    assert!(v.get("blocked").is_none());
}

// --- TOML config round-trip ---

#[test]
fn toml_config_drives_zone_checker() {
    use agent007_zones::config::ZoneConfig;
    let toml_str = r#"
[zones]
forbidden    = ["secrets/", ".env", "*.pem"]
readonly     = ["src/auth/"]
sensitive    = ["src/crypto/"]
unrestricted = ["src/", "tests/"]
"#;
    let config = ZoneConfig::from_toml(toml_str).unwrap();
    let checker = ZoneChecker::new(&config).unwrap();

    assert_eq!(checker.zone_for(Path::new(".env")), ZoneLevel::Forbidden);
    assert_eq!(
        checker.zone_for(Path::new("src/auth/login.rs")),
        ZoneLevel::Readonly
    );
    assert_eq!(
        checker.zone_for(Path::new("src/crypto/hash.rs")),
        ZoneLevel::Sensitive
    );
    assert_eq!(
        checker.zone_for(Path::new("src/utils.rs")),
        ZoneLevel::Unrestricted
    );
}
