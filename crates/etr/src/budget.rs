//! Work limits are applied before compaction. Cooperative deadlines cannot
//! interrupt a blocked OS filesystem syscall and are not an OS sandbox.
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
