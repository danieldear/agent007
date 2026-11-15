//! Work limits are applied before compaction. Cooperative deadlines cannot
//! interrupt a blocked OS filesystem syscall and are not an OS sandbox.
use anyhow::Context;
use std::{
    io::{self, Read},
    path::{Path, PathBuf},
    time::{Duration, Instant},
};
pub const FILE_BYTES: u64 = 8 * 1024 * 1024;
pub const TOTAL_BYTES: u64 = 32 * 1024 * 1024;
pub const ENTRIES: usize = 10_000;
pub const RESULTS: usize = 512;
pub const OUTPUT_BYTES: usize = 256 * 1024;
pub fn read(path: impl AsRef<Path>) -> io::Result<Vec<u8>> {
    let path = path.as_ref();
    if std::fs::symlink_metadata(path)?.file_type().is_symlink() {
        return Err(io::Error::other("Symlink input is not allowed"));
    }
    let mut b = Vec::new();
    std::fs::File::open(path)?
        .take(FILE_BYTES + 1)
        .read_to_end(&mut b)?;
    if b.len() as u64 > FILE_BYTES {
        return Err(io::Error::other("Tool file exceeds 8 MiB budget"));
    }
    Ok(b)
}
pub fn read_to_string(path: impl AsRef<Path>) -> io::Result<String> {
    String::from_utf8(read(path)?).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))
}
pub fn files(root: &Path) -> (Vec<PathBuf>, bool) {
    let start = Instant::now();
    let mut bytes = 0;
    let mut out = Vec::new();
    let mut truncated = false;
    for (n, e) in ignore::WalkBuilder::new(root)
        .follow_links(false)
        .require_git(false)
        .max_depth(Some(32))
        .build()
        .enumerate()
    {
        if n >= ENTRIES || start.elapsed() > Duration::from_secs(2) {
            truncated = true;
            break;
        }
        let Ok(e) = e else { continue };
        if !e.file_type().is_some_and(|t| t.is_file()) {
            continue;
        }
        let Ok(m) = e.metadata() else { continue };
        if m.len() > FILE_BYTES {
            truncated = true;
            continue;
        }
        bytes += m.len();
        if bytes > TOTAL_BYTES {
            truncated = true;
            break;
        }
        out.push(e.path().to_path_buf());
    }
    (out, truncated)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn oversized_files_are_rejected_before_full_read() {
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("big");
        std::fs::File::create(&p)
            .unwrap()
            .set_len(FILE_BYTES + 1)
            .unwrap();
        assert!(read(p).is_err());
    }
    #[cfg(unix)]
    #[test]
    fn walker_never_follows_cycles_or_outside_links() {
        let d = tempfile::tempdir().unwrap();
        let root = d.path().join("p");
        std::fs::create_dir(&root).unwrap();
        std::fs::write(root.join("real"), "x").unwrap();
        std::os::unix::fs::symlink(d.path(), root.join("loop")).unwrap();
        let (f, _) = files(&root);
        assert_eq!(f.len(), 1);
    }
}
pub fn csv(path: &str) -> anyhow::Result<csv::Reader<std::io::Cursor<Vec<u8>>>> {
    let bytes = read(path).with_context(|| format!("cannot read CSV: {path}"))?;
    let mut reader = csv::Reader::from_reader(std::io::Cursor::new(bytes));
    if reader.headers()?.len() > 256 {
        anyhow::bail!("CSV exceeds column budget");
    }
    Ok(reader)
}
pub fn cells(total: &mut usize, n: usize) -> anyhow::Result<()> {
    *total = total.saturating_add(n);
    if *total > 100_000 {
        anyhow::bail!("Table exceeds 100000-cell work budget");
    }
    Ok(())
}
pub fn push(
    out: &mut Vec<serde_json::Value>,
    value: serde_json::Value,
    bytes: &mut usize,
) -> anyhow::Result<()> {
    let n = serde_json::to_vec(&value)?.len();
    if out.len() >= RESULTS || bytes.saturating_add(n) > OUTPUT_BYTES {
        anyhow::bail!("Join exceeds result budget; narrow the inputs");
    }
    *bytes += n;
    out.push(value);
    Ok(())
}
#[cfg(test)]
mod additional_regressions {
    use super::*;
    use serde_json::json;
    #[test]
    fn csv_read_errors_include_the_source_path() {
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("missing.csv");
        let e = csv(p.to_str().unwrap()).err().unwrap();
        assert!(e.to_string().contains(p.to_str().unwrap()));
    }
    #[test]
    fn csv_entry_points_cannot_bypass_file_limits() {
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("big.csv");
        std::fs::File::create(&p)
            .unwrap()
            .set_len(FILE_BYTES + 1)
            .unwrap();
        for tool in ["etr.csv_slice", "etr.table_select", "etr.table_stats"] {
            assert!(crate::l1::dispatch(tool, &json!({"path":p})).is_err());
        }
    }
    #[test]
    fn cartesian_joins_stop_before_unbounded_accumulation() {
        let rows = vec![json!({"id":1}); 100];
        let r = crate::l1::join_on_key::run(
            &json!({"left_key":"id","right_key":"id","left_rows":rows,"right_rows":rows}),
        );
        assert!(r.unwrap_err().to_string().contains("budget"));
    }
    #[test]
    fn json_selectors_and_inline_path_fields_are_data_not_files() {
        let d = tempfile::tempdir().unwrap();
        std::fs::write(d.path().join("data.json"), r#"{"a":{"path":"/data"}}"#).unwrap();
        let dispatcher = crate::EtrDispatcher::new(d.path().to_path_buf());
        let r = dispatcher.call(crate::EtrCallRequest {
            tool: "etr.json_extract".into(),
            input: json!({"path":"data.json","jq_path":".a.path"}),
            compact: false,
        });
        assert!(matches!(r.status, crate::EtrStatus::Ok), "{:?}", r.error);
        assert_eq!(r.output["value"], "/data");
        let policy = crate::policy::PolicyEngine::new(d.path().to_path_buf());
        assert!(policy
            .prepare(&json!({"baseline":{"path":42},"candidate":{"path":43}}))
            .is_ok());
    }
}
