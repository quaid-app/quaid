use std::env;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::time::Duration;

const MODEL_ID: &str = "BAAI/bge-small-en-v1.5";
const MODEL_FILES: [&str; 3] = ["config.json", "tokenizer.json", "model.safetensors"];

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
    for file_name in MODEL_FILES {
        let source = model_dir.join(file_name);
        if !source.is_file() {
            return Err(format!("missing {file_name} in {}", model_dir.display()));
        }

        let destination = out_dir.join(file_name);
        fs::copy(&source, &destination)
            .map_err(|error| format!("copy {}: {error}", source.display()))?;
    }

    Ok(())
}

fn download_model_files(out_dir: &Path) -> Result<(), String> {
    let client = reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(300))
        .user_agent("gigabrain-build/0.9.1")
        .build()
        .map_err(|error| format!("build download client: {error}"))?;

    for file_name in MODEL_FILES {
        let url = format!("https://huggingface.co/{MODEL_ID}/resolve/main/{file_name}");
        let mut response = client
            .get(&url)
            .send()
            .and_then(reqwest::blocking::Response::error_for_status)
            .map_err(|error| format!("download {url}: {error}"))?;

        let destination = out_dir.join(file_name);
        let mut file = fs::File::create(&destination)
            .map_err(|error| format!("create {}: {error}", destination.display()))?;
        io::copy(&mut response, &mut file)
            .map_err(|error| format!("write {}: {error}", destination.display()))?;
    }

    Ok(())
}

fn find_local_model_dir() -> Option<PathBuf> {
    candidate_model_dirs().into_iter().find(|path| {
        MODEL_FILES
            .iter()
            .all(|file_name| path.join(file_name).is_file())
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
