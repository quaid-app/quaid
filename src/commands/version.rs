use anyhow::Result;

pub fn run() -> Result<()> {
    println!("quaid {}", env!("CARGO_PKG_VERSION"));
    Ok(())
}
