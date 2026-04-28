use std::path::{Path, PathBuf};
use std::sync::OnceLock;

#[cfg(windows)]
const EXE_SUFFIX: &str = ".exe";
#[cfg(not(windows))]
const EXE_SUFFIX: &str = "";

pub fn quaid_bin() -> &'static Path {
    static BIN_PATH: OnceLock<PathBuf> = OnceLock::new();
    BIN_PATH
        .get_or_init(|| {
            if let Some(path) = option_env!("CARGO_BIN_EXE_quaid") {
                let candidate = PathBuf::from(path);
                if candidate.is_file() {
                    return candidate;
                }
            }

            let bin = format!("quaid{EXE_SUFFIX}");
            let current_exe = std::env::current_exe().expect("locate current test executable");
            let mut candidates = Vec::new();
            if let Some(dir) = current_exe.parent() {
                candidates.push(dir.join(&bin));
                if let Some(parent) = dir.parent() {
                    candidates.push(parent.join(&bin));
                    candidates.push(parent.join("deps").join(&bin));
                }
            }

            let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
            candidates.push(manifest_dir.join("target").join("debug").join(&bin));
            candidates.push(
                manifest_dir
                    .join("target")
                    .join("debug")
                    .join("deps")
                    .join(&bin),
            );
            candidates.push(manifest_dir.join("target").join("release").join(&bin));
            candidates.push(
                manifest_dir
                    .join("target")
                    .join("release")
                    .join("deps")
                    .join(&bin),
            );

            candidates
                .into_iter()
                .find(|candidate| candidate.is_file())
                .unwrap_or_else(|| {
                    panic!(
                        "failed to locate quaid test binary via CARGO_BIN_EXE_quaid or current_exe fallback"
                    )
                })
        })
        .as_path()
}
