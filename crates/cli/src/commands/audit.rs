// crates/cli/src/commands/audit.rs
use anyhow::Result;
use chrono::{DateTime, Duration, Utc};
use globset::{Glob, GlobMatcher};
use serde_json::Value;
use std::sync::Arc;

use super::run::agent007_home;
use crate::config::Config;
use agent007_zones::AuditLogger;

/// Parse a duration string like "24h", "1h", "30m" into a chrono::Duration.
fn parse_duration(s: &str) -> Result<Duration> {
    if let Some(h) = s.strip_suffix('h') {
        let n: i64 = h
            .parse()
            .map_err(|_| anyhow::anyhow!("invalid duration: {}", s))?;
        Ok(Duration::hours(n))
    } else if let Some(m) = s.strip_suffix('m') {
        let n: i64 = m
            .parse()
            .map_err(|_| anyhow::anyhow!("invalid duration: {}", s))?;
        Ok(Duration::minutes(n))
    } else {
        Err(anyhow::anyhow!(
            "unsupported duration format: '{}' (use Nh or Nm)",
            s
        ))
    }
}

pub async fn execute(
    _config: Arc<Config>,
    last: Option<String>,
    agent_filter: Option<String>,
    path_filter: Option<String>,
    blocked_only: bool,
) -> Result<()> {
    let home = agent007_home();
    let log_path = home.join("audit").join("audit.log");
    let logger = AuditLogger::new(&log_path);

    let lines = logger.read_lines()?;

    // Parse --last into a cutoff timestamp
    let cutoff: Option<DateTime<Utc>> = match last {
        Some(ref s) => {
            let dur = parse_duration(s)?;
            Some(Utc::now() - dur)
        }
        None => None,
    };

    // Compile --path glob
    let path_matcher: Option<GlobMatcher> = match path_filter {
        Some(ref p) => {
            let glob =
                Glob::new(p).map_err(|e| anyhow::anyhow!("invalid path glob '{}': {}", p, e))?;
            Some(glob.compile_matcher())
        }
        None => None,
    };

    let mut count = 0usize;

    for line in &lines {
        let entry: Value = match serde_json::from_str(line) {
            Ok(v) => v,
            Err(_) => continue, // skip malformed lines
        };

        // Filter: --last
        if let Some(cutoff_ts) = cutoff {
            if let Some(ts_str) = entry.get("ts").and_then(|v| v.as_str()) {
                let ts = ts_str
                    .parse::<DateTime<Utc>>()
                    .unwrap_or(DateTime::<Utc>::from_timestamp(0, 0).unwrap());
                if ts < cutoff_ts {
                    continue;
                }
            }
        }

        // Filter: --agent
        if let Some(ref agent) = agent_filter {
            let entry_agent = entry.get("agent").and_then(|v| v.as_str()).unwrap_or("");
            if entry_agent != agent.as_str() {
                continue;
            }
        }

        // Filter: --blocked
        if blocked_only {
            let is_blocked = entry
                .get("blocked")
                .and_then(|v| v.as_bool())
                .unwrap_or(false);
            if !is_blocked {
                continue;
            }
        }

        // Filter: --path glob
        if let Some(ref matcher) = path_matcher {
            let entry_path = entry.get("path").and_then(|v| v.as_str()).unwrap_or("");
            if !matcher.is_match(entry_path) {
                continue;
            }
        }

        // Print matching entry
        println!("{}", line);
        count += 1;
    }

    if count == 0 {
        println!("(no audit entries match the given filters)");
    } else {
        eprintln!("{} entries", count);
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_duration_hours() {
        let d = parse_duration("24h").unwrap();
        assert_eq!(d, Duration::hours(24));
    }

    #[test]
    fn parse_duration_minutes() {
        let d = parse_duration("30m").unwrap();
        assert_eq!(d, Duration::minutes(30));
    }

    #[test]
    fn parse_duration_invalid_returns_err() {
        assert!(parse_duration("5d").is_err());
        assert!(parse_duration("abc").is_err());
        assert!(parse_duration("").is_err());
    }
}
