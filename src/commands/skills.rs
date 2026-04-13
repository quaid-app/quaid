use anyhow::Result;
use clap::Subcommand;

#[derive(Subcommand)]
pub enum SkillsAction {
    Doctor,
    List,
}

pub fn run(_action: SkillsAction) -> Result<()> {
    todo!("skills: manage and inspect brain skills")
}
