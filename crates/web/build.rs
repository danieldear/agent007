use std::env;
use std::ffi::OsStr;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::SystemTime;

fn main() {
    println!("cargo:rerun-if-env-changed=AGENT007_SKIP_FRONTEND_BUILD");
    println!("cargo:rerun-if-changed=frontend/package.json");
    println!("cargo:rerun-if-changed=frontend/package-lock.json");
    println!("cargo:rerun-if-changed=frontend/index.html");
    println!("cargo:rerun-if-changed=frontend/vite.config.js");
    println!("cargo:rerun-if-changed=frontend/src");
    println!("cargo:rerun-if-changed=static/dist/index.html");

    if skip_requested() {
        println!("cargo:warning=Skipping frontend build (AGENT007_SKIP_FRONTEND_BUILD=1)");
        return;
    }

    let manifest_dir =
        PathBuf::from(env::var("CARGO_MANIFEST_DIR").expect("missing CARGO_MANIFEST_DIR"));
    let frontend_dir = manifest_dir.join("frontend");
    let dist_index = manifest_dir.join("static").join("dist").join("index.html");

    if !should_build_frontend(&frontend_dir, &dist_index) {
        return;
    }

    let npm = npm_command();
    let node_modules = frontend_dir.join("node_modules");

    if !node_modules.exists() {
        run(
            npm,
            &["ci", "--silent"],
            &frontend_dir,
            "install frontend dependencies",
        );
    }

    run(
        npm,
        &["run", "build"],
        &frontend_dir,
        "build frontend assets",
    );

    if !dist_index.exists() {
        panic!(
            "frontend build finished but '{}' was not generated",
            dist_index.display()
        );
    }
}

fn skip_requested() -> bool {
    env::var("AGENT007_SKIP_FRONTEND_BUILD")
        .map(|v| {
            let normalized = v.trim().to_ascii_lowercase();
            normalized == "1" || normalized == "true" || normalized == "yes"
        })
        .unwrap_or(false)
}

fn should_build_frontend(frontend_dir: &Path, dist_index: &Path) -> bool {
    if !dist_index.exists() {
        return true;
    }

    let dist_mtime = modified_time(dist_index);
    let latest_source_mtime = latest_frontend_source_mtime(frontend_dir);

    latest_source_mtime > dist_mtime
}

fn latest_frontend_source_mtime(frontend_dir: &Path) -> SystemTime {
    let mut latest = SystemTime::UNIX_EPOCH;

    for file in [
        "package.json",
        "package-lock.json",
        "index.html",
        "vite.config.js",
    ] {
        let path = frontend_dir.join(file);
        latest = latest.max(modified_time(&path));
    }

    latest = latest.max(latest_mtime_recursive(&frontend_dir.join("src")));
    latest
}

fn latest_mtime_recursive(path: &Path) -> SystemTime {
    let mut latest = SystemTime::UNIX_EPOCH;
    if !path.exists() {
        return latest;
    }

    if path.is_file() {
        return modified_time(path);
    }

    let entries =
        fs::read_dir(path).unwrap_or_else(|e| panic!("failed to read '{}': {e}", path.display()));
    for entry in entries {
        let entry = entry.unwrap_or_else(|e| panic!("failed to read dir entry: {e}"));
        let p = entry.path();
        if p.file_name() == Some(OsStr::new("node_modules")) {
            continue;
        }
        latest = latest.max(latest_mtime_recursive(&p));
    }
    latest
}

fn modified_time(path: &Path) -> SystemTime {
    fs::metadata(path)
        .and_then(|m| m.modified())
        .unwrap_or(SystemTime::UNIX_EPOCH)
}

fn run(cmd: &str, args: &[&str], cwd: &Path, action: &str) {
    let status = Command::new(cmd).args(args).current_dir(cwd).status();
    match status {
        Ok(s) if s.success() => {}
        Ok(s) => panic!(
            "failed to {action}: command '{}' {:?} exited with status {s}",
            cmd, args
        ),
        Err(e) => panic!(
            "failed to {action}: could not execute '{}' {:?}: {e}",
            cmd, args
        ),
    }
}

fn npm_command() -> &'static str {
    if cfg!(windows) {
        "npm.cmd"
    } else {
        "npm"
    }
}
