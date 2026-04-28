use anyhow::Result;

pub fn run() -> Result<()> {
    println!("quaid {}", env!("CARGO_PKG_VERSION"));
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn run_succeeds_and_returns_ok() {
        assert!(run().is_ok());
    }
}
