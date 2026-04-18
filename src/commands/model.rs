use anyhow::Result;

pub struct KnownModel {
    pub alias: &'static str,
    pub model_id: &'static str,
    pub dim: usize,
    pub size_mb: u32,
    pub notes: &'static str,
}

pub const KNOWN_MODELS: &[KnownModel] = &[
    KnownModel {
        alias: "small",
        model_id: "BAAI/bge-small-en-v1.5",
        dim: 384,
        size_mb: 130,
        notes: "default, fastest",
    },
    KnownModel {
        alias: "base",
        model_id: "BAAI/bge-base-en-v1.5",
        dim: 768,
        size_mb: 440,
        notes: "",
    },
    KnownModel {
        alias: "medium",
        model_id: "BAAI/bge-base-en-v1.5",
        dim: 768,
        size_mb: 440,
        notes: "alias for base",
    },
    KnownModel {
        alias: "large",
        model_id: "BAAI/bge-large-en-v1.5",
        dim: 1024,
        size_mb: 1340,
        notes: "",
    },
    KnownModel {
        alias: "m3",
        model_id: "BAAI/bge-m3",
        dim: 1024,
        size_mb: 2270,
        notes: "multilingual",
    },
    KnownModel {
        alias: "max",
        model_id: "BAAI/bge-m3",
        dim: 1024,
        size_mb: 2270,
        notes: "alias for m3",
    },
];

pub fn run(json: bool) -> Result<()> {
    if json {
        print_json()
    } else {
        print_table()
    }
}

fn print_table() -> Result<()> {
    println!(
        "{:<10} {:<25} {:>5} {:>8}  NOTES",
        "ALIAS", "MODEL_ID", "DIM", "SIZE_MB"
    );
    for m in KNOWN_MODELS {
        println!(
            "{:<10} {:<25} {:>5} {:>8}  {}",
            m.alias, m.model_id, m.dim, m.size_mb, m.notes
        );
    }
    Ok(())
}

fn print_json() -> Result<()> {
    let entries: Vec<serde_json::Value> = KNOWN_MODELS
        .iter()
        .map(|m| {
            serde_json::json!({
                "alias": m.alias,
                "model_id": m.model_id,
                "dim": m.dim,
                "size_mb": m.size_mb,
                "notes": m.notes,
            })
        })
        .collect();
    println!("{}", serde_json::to_string_pretty(&entries)?);
    Ok(())
}
