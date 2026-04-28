use std::path::{Path, PathBuf};
use std::sync::OnceLock;

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

            let current_exe = std::env::current_exe().expect("locate current test executable");
            let mut candidates = Vec::new();
            if let Some(dir) = current_exe.parent() {
                candidates.push(dir.join("quaid.exe"));
                if let Some(parent) = dir.parent() {
                    candidates.push(parent.join("quaid.exe"));
                    candidates.push(parent.join("deps").join("quaid.exe"));
                }
            }

            let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
            candidates.push(manifest_dir.join("target").join("debug").join("quaid.exe"));
            candidates.push(
                manifest_dir
                    .join("target")
                    .join("debug")
                    .join("deps")
                    .join("quaid.exe"),
            );
            candidates.push(manifest_dir.join("target").join("release").join("quaid.exe"));
            candidates.push(
                manifest_dir
                    .join("target")
                    .join("release")
                    .join("deps")
                    .join("quaid.exe"),
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
