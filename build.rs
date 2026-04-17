use std::env;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::time::Duration;

use sha2::{Digest, Sha256};

const MODEL_ID: &str = "BAAI/bge-small-en-v1.5";
/// Pinned Hugging Face revision for reproducible builds.
const MODEL_REVISION: &str = "5c38ec7c405ec4b44b94cc5a9bb96e735b38267a";

/// Model files and their expected SHA-256 digests for integrity verification.
const MODEL_FILES: [(&str, &str); 3] = [
    (
        "config.json",
        "094f8e891b932f2000c92cfc663bac4c62069f5d8af5b5278c4306aef3084750",
    ),
    (
        "tokenizer.json",
        "d241a60d5e8f04cc1b2b3e9ef7a4921b27bf526d9f6050ab90f9267a1f9e5c66",
    ),
    (
        "model.safetensors",
        "3c9f31665447c8911517620762200d2245a2518d6e7208acc78cd9db317e21ad",
    ),
];

fn main() {
    println!("cargo:rerun-if-env-changed=GBRAIN_EMBEDDED_MODEL_DIR");
    println!("cargo:rerun-if-env-changed=GBRAIN_MODEL_DIR");

    if env::var_os("CARGO_FEATURE_EMBEDDED_MODEL").is_some() {
        prepare_embedded_model().unwrap_or_else(|error| {
            panic!("failed to prepare embedded BGE-small assets: {error}");
        });
    }
}

fn prepare_embedded_model() -> Result<(), String> {
    let out_dir = PathBuf::from(env::var("OUT_DIR").map_err(|error| error.to_string())?)
        .join("embedded-model");
    fs::create_dir_all(&out_dir).map_err(|error| format!("create output dir: {error}"))?;

    if let Some(model_dir) = find_local_model_dir() {
        copy_model_files(&model_dir, &out_dir)?;
    } else {
        download_model_files(&out_dir)?;
    }

    println!(
        "cargo:rustc-env=GBRAIN_EMBEDDED_CONFIG_PATH={}",
        out_dir.join("config.json").display()
    );
    println!(
        "cargo:rustc-env=GBRAIN_EMBEDDED_TOKENIZER_PATH={}",
        out_dir.join("tokenizer.json").display()
    );
    println!(
        "cargo:rustc-env=GBRAIN_EMBEDDED_MODEL_PATH={}",
        out_dir.join("model.safetensors").display()
    );

    Ok(())
}

fn copy_model_files(model_dir: &Path, out_dir: &Path) -> Result<(), String> {
    for (file_name, expected_hash) in MODEL_FILES {
        let source = model_dir.join(file_name);
        if !source.is_file() {
            return Err(format!("missing {file_name} in {}", model_dir.display()));
        }

        let destination = out_dir.join(file_name);
        fs::copy(&source, &destination)
            .map_err(|error| format!("copy {}: {error}", source.display()))?;

        verify_sha256(&destination, expected_hash)?;
    }

    Ok(())
}

fn download_model_files(out_dir: &Path) -> Result<(), String> {
    let client = reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(300))
        .user_agent("gigabrain-build/0.9.1")
        .build()
        .map_err(|error| format!("build download client: {error}"))?;

    for (file_name, expected_hash) in MODEL_FILES {
        let url = format!("https://huggingface.co/{MODEL_ID}/resolve/{MODEL_REVISION}/{file_name}");
        let temp_destination = out_dir.join(format!("{file_name}.download"));
        let mut response = client
            .get(&url)
            .send()
            .and_then(reqwest::blocking::Response::error_for_status)
            .map_err(|error| format!("download {url}: {error}"))?;

        let mut file = fs::File::create(&temp_destination)
            .map_err(|error| format!("create {}: {error}", temp_destination.display()))?;
        io::copy(&mut response, &mut file)
            .map_err(|error| format!("write {}: {error}", temp_destination.display()))?;

        verify_sha256(&temp_destination, expected_hash)?;

        let destination = out_dir.join(file_name);
        fs::rename(&temp_destination, &destination).map_err(|error| {
            format!(
                "rename {} -> {}: {error}",
                temp_destination.display(),
                destination.display()
            )
        })?;
    }

    Ok(())
}

fn find_local_model_dir() -> Option<PathBuf> {
    candidate_model_dirs().into_iter().find(|path| {
        MODEL_FILES
            .iter()
            .all(|(file_name, _)| path.join(file_name).is_file())
    })
}

fn candidate_model_dirs() -> Vec<PathBuf> {
    let mut dirs = Vec::new();

    for env_var in ["GBRAIN_EMBEDDED_MODEL_DIR", "GBRAIN_MODEL_DIR"] {
        if let Some(path) = env::var_os(env_var) {
            dirs.push(PathBuf::from(path));
        }
    }

    if let Some(home) = home_dir() {
        dirs.push(
            home.join(".gbrain")
                .join("models")
                .join("bge-small-en-v1.5"),
        );

        let snapshots = home
            .join(".cache")
            .join("huggingface")
            .join("hub")
            .join("models--BAAI--bge-small-en-v1.5")
            .join("snapshots");

        if let Ok(entries) = fs::read_dir(snapshots) {
            let mut snapshot_dirs: Vec<PathBuf> = entries
                .filter_map(Result::ok)
                .map(|entry| entry.path())
                .collect();
            snapshot_dirs.sort();
            dirs.extend(snapshot_dirs);
        }
    }

    dirs
}

fn home_dir() -> Option<PathBuf> {
    env::var_os("HOME")
        .or_else(|| env::var_os("USERPROFILE"))
        .map(PathBuf::from)
}

fn verify_sha256(path: &Path, expected: &str) -> Result<(), String> {
    let mut file =
        fs::File::open(path).map_err(|e| format!("open {} for hash: {e}", path.display()))?;
    let mut hasher = Sha256::new();
    io::copy(&mut file, &mut hasher)
        .map_err(|e| format!("read {} for hash: {e}", path.display()))?;
    let actual = format!("{:x}", hasher.finalize());

    if actual != expected {
        return Err(format!(
            "SHA-256 mismatch for {}: expected {expected}, got {actual}",
            path.display()
        ));
    }
    Ok(())
}
