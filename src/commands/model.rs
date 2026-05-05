use anyhow::Result;
use clap::Subcommand;

use crate::core::conversation::model_lifecycle::{download_model, ConsoleProgressReporter};

#[derive(Clone, Debug, PartialEq, Eq, Subcommand)]
pub enum ModelAction {
    /// Download a local SLM into the Quaid model cache
    Pull { alias: String },
}

pub fn run(action: ModelAction) -> Result<()> {
    match action {
        ModelAction::Pull { alias } => {
            let mut progress = ConsoleProgressReporter;
            let cache_dir = tokio::task::block_in_place(|| download_model(&alias, &mut progress))?;
            println!("Model cached at {}", cache_dir.display());
            Ok(())
        }
    }
}
