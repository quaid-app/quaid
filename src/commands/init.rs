use std::path::Path;

use anyhow::Result;

use crate::core::{db, inference};

pub fn run(path: &str, requested_model: &inference::ModelConfig) -> Result<()> {
    let db_path = Path::new(path);

    if db_path.exists() {
        db::init(path, &inference::resolve_model("small"))?;
        println!("Database already exists at {path}");
        return Ok(());
    }

    db::init(path, requested_model)?;
    println!("Brain initialized at {path}");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn init_creates_new_database_and_succeeds() {
        let dir = tempfile::TempDir::new().unwrap();
        let db_path = dir.path().join("test_brain.db");
        let path_str = db_path.to_str().unwrap();

        let result = run(path_str, &inference::resolve_model("small"));
        assert!(result.is_ok());
        assert!(db_path.exists());
    }

    #[test]
    fn init_on_existing_database_succeeds_without_reinit() {
        let dir = tempfile::TempDir::new().unwrap();
        let db_path = dir.path().join("test_brain.db");
        let path_str = db_path.to_str().unwrap();

        // Create it first
        run(path_str, &inference::resolve_model("small")).unwrap();
        let metadata_before = std::fs::metadata(&db_path).unwrap();
        let size_before = metadata_before.len();

        // Run again — should be a no-op
        let result = run(path_str, &inference::resolve_model("large"));
        assert!(result.is_ok());

        // File should still exist, size unchanged (no schema rewrite)
        let size_after = std::fs::metadata(&db_path).unwrap().len();
        assert_eq!(size_before, size_after);
    }

    #[test]
    fn init_rejects_nonexistent_parent_directory() {
        let result = run("/nonexistent/dir/brain.db", &inference::resolve_model("small"));
        assert!(result.is_err());
    }
}
