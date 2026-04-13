use anyhow::Result;

pub fn run() -> Result<()> {
    println!("gbrain {}", env!("CARGO_PKG_VERSION"));
    Ok(())
}
