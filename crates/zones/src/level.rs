// crates/zones/src/level.rs

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ZoneLevel {
    Unrestricted,
    Readonly,
    Sensitive,
    Forbidden,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FileOp {
    Read,
    Write,
    Delete,
}

impl ZoneLevel {
    /// Numeric priority — higher value = more restrictive.
    /// Used by ZoneChecker::zone_for when multiple patterns match.
    pub fn priority(self) -> u8 {
        match self {
            ZoneLevel::Unrestricted => 0,
            ZoneLevel::Readonly => 1,
            ZoneLevel::Sensitive => 2,
            ZoneLevel::Forbidden => 3,
        }
    }

    /// Most restrictive level wins (used when merging multiple ZoneConfigs).
    pub fn most_restrictive(self, other: ZoneLevel) -> ZoneLevel {
        if other.priority() > self.priority() {
            other
        } else {
            self
        }
    }

    /// Returns the string label used in audit log JSON.
    pub fn as_str(self) -> &'static str {
        match self {
            ZoneLevel::Unrestricted => "unrestricted",
            ZoneLevel::Readonly => "readonly",
            ZoneLevel::Sensitive => "sensitive",
            ZoneLevel::Forbidden => "forbidden",
        }
    }
}

impl FileOp {
    /// Returns the string label used in audit log JSON.
    pub fn as_str(self) -> &'static str {
        match self {
            FileOp::Read => "read",
            FileOp::Write => "write",
            FileOp::Delete => "delete",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn zone_level_priority_ordering() {
        assert!(ZoneLevel::Unrestricted.priority() < ZoneLevel::Readonly.priority());
        assert!(ZoneLevel::Readonly.priority() < ZoneLevel::Sensitive.priority());
        assert!(ZoneLevel::Sensitive.priority() < ZoneLevel::Forbidden.priority());
    }

    #[test]
    fn most_restrictive_picks_higher_priority() {
        assert_eq!(
            ZoneLevel::Unrestricted.most_restrictive(ZoneLevel::Forbidden),
            ZoneLevel::Forbidden
        );
        assert_eq!(
            ZoneLevel::Sensitive.most_restrictive(ZoneLevel::Readonly),
            ZoneLevel::Sensitive
        );
        assert_eq!(
            ZoneLevel::Readonly.most_restrictive(ZoneLevel::Readonly),
            ZoneLevel::Readonly
        );
    }

    #[test]
    fn zone_level_as_str() {
        assert_eq!(ZoneLevel::Unrestricted.as_str(), "unrestricted");
        assert_eq!(ZoneLevel::Readonly.as_str(), "readonly");
        assert_eq!(ZoneLevel::Sensitive.as_str(), "sensitive");
        assert_eq!(ZoneLevel::Forbidden.as_str(), "forbidden");
    }

    #[test]
    fn file_op_as_str() {
        assert_eq!(FileOp::Read.as_str(), "read");
        assert_eq!(FileOp::Write.as_str(), "write");
        assert_eq!(FileOp::Delete.as_str(), "delete");
    }
}
