use include_dir::{include_dir, Dir};
use std::path::PathBuf;

/// Inline single-page fallback dashboard HTML served at `GET /`.
/// Source lives at `static/index.html`; embedded at compile time.
pub const DASHBOARD_HTML: &str = include_str!("../static/index.html");

/// Embedded static directory (includes `dist/` when present at build time).
static EMBEDDED_STATIC: Dir<'_> = include_dir!("$CARGO_MANIFEST_DIR/static");

fn dist_root_on_disk() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("static/dist")
}

fn normalize_rel_path(path: &str) -> Option<&str> {
    let trimmed = path.trim_start_matches('/');
    if trimmed.is_empty() || trimmed.contains("..") {
        return None;
    }
    Some(trimmed)
}

/// Load a file from `static/dist`, preferring on-disk assets for local development,
/// and falling back to embedded assets for installed binaries.
pub fn load_dist_file(path: &str) -> Option<Vec<u8>> {
    let rel = normalize_rel_path(path)?;
    let disk_path = dist_root_on_disk().join(rel);
    if disk_path.exists() {
        return std::fs::read(disk_path).ok();
    }
    let embedded_rel = format!("dist/{rel}");
    EMBEDDED_STATIC
        .get_file(embedded_rel.as_str())
        .map(|file| file.contents().to_vec())
}

/// Load dashboard index from dist bundle if available.
pub fn load_dist_index_html() -> Option<String> {
    load_dist_file("index.html").and_then(|bytes| String::from_utf8(bytes).ok())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn first_asset_path(index_html: &str) -> Option<String> {
        let marker = "/assets/";
        let start = index_html.find(marker)?;
        let after = &index_html[start..];
        let end = after.find('"')?;
        Some(after[..end].trim_start_matches('/').to_string())
    }

    #[test]
    fn dist_index_is_available() {
        let index = load_dist_index_html();
        if std::env::var("AGENT007_SKIP_FRONTEND_BUILD")
            .map(|v| matches!(v.trim(), "1" | "true" | "TRUE" | "yes" | "YES"))
            .unwrap_or(false)
        {
            assert!(
                index.is_some() || !DASHBOARD_HTML.trim().is_empty(),
                "either dist index or fallback dashboard HTML must be available"
            );
        } else {
            assert!(index.is_some(), "dist index must be available");
        }
    }

    #[test]
    fn dist_asset_referenced_by_index_is_available() {
        let Some(index) = load_dist_index_html() else {
            return;
        };
        let asset =
            first_asset_path(&index).expect("index should reference at least one /assets file");
        assert!(
            load_dist_file(&asset).is_some(),
            "referenced asset '{asset}' should be available"
        );
    }

    #[test]
    fn load_dist_file_rejects_path_traversal() {
        assert!(load_dist_file("../etc/passwd").is_none());
    }
}
