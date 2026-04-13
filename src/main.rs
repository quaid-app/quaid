use anyhow::Result;
use clap::{Parser, Subcommand};

use std::path::PathBuf;

mod commands;
mod core;
mod mcp;

/// Platform-safe default database path.
/// Uses `$HOME/brain.db` on all platforms, with proper home-dir resolution.
/// Falls back to `./brain.db` if the home directory cannot be determined.
fn default_db_path() -> String {
    std::env::var("HOME")
        .or_else(|_| std::env::var("USERPROFILE"))
        .map(|home| {
            let mut p = PathBuf::from(home);
            p.push("brain.db");
            p.to_string_lossy().into_owned()
        })
        .unwrap_or_else(|_| "brain.db".to_owned())
}

#[derive(Parser)]
#[command(
    name = "gbrain",
    version,
    about = "Personal knowledge brain — SQLite + FTS5 + local vector embeddings"
)]
struct Cli {
    /// Path to brain database file [default: ./brain.db]
    #[arg(long, env = "GBRAIN_DB", global = true)]
    db: Option<String>,

    /// Output as JSON
    #[arg(long, global = true)]
    json: bool,

    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Initialise a new brain database
    Init,
    /// Read a page by slug
    Get { slug: String },
    /// Write or update a page (reads from stdin)
    Put { slug: String },
    /// List pages with optional filters
    List {
        #[arg(long)]
        wing: Option<String>,
        #[arg(long)]
        r#type: Option<String>,
        #[arg(long, default_value = "50")]
        limit: u32,
    },
    /// Full-text search
    Search {
        query: String,
        #[arg(long)]
        wing: Option<String>,
        #[arg(long, default_value = "10")]
        limit: u32,
    },
    /// Semantic / hybrid query
    Query {
        query: String,
        #[arg(long, default_value = "auto")]
        depth: String,
        #[arg(long, default_value = "4000")]
        token_budget: u32,
        #[arg(long)]
        wing: Option<String>,
    },
    /// Ingest a source document
    Ingest {
        path: String,
        #[arg(long)]
        force: bool,
    },
    /// Import a markdown directory
    Import {
        path: String,
        #[arg(long)]
        validate_only: bool,
    },
    /// Export brain to markdown directory
    Export {
        path: String,
        #[arg(long)]
        raw: bool,
        #[arg(long)]
        import_id: Option<String>,
    },
    /// Generate or refresh embeddings
    Embed {
        #[arg(long)]
        all: bool,
        #[arg(long)]
        stale: bool,
    },
    /// Create a typed temporal link between pages
    Link {
        from: String,
        to: String,
        #[arg(long, default_value = "related")]
        relationship: String,
        #[arg(long)]
        valid_from: Option<String>,
        #[arg(long)]
        valid_until: Option<String>,
    },
    /// Close a temporal link by ID
    Unlink { link_id: u64 },
    /// List backlinks for a page
    Backlinks {
        slug: String,
        #[arg(long)]
        temporal: Option<String>,
    },
    /// Tag a page
    Tag { slug: String, tags: Vec<String> },
    /// Untag a page
    Untag { slug: String, tags: Vec<String> },
    /// Show timeline entries for a page
    Timeline {
        slug: String,
        #[arg(long, default_value = "20")]
        limit: u32,
    },
    /// N-hop graph neighbourhood
    Graph {
        slug: String,
        #[arg(long, default_value = "2")]
        depth: u32,
        #[arg(long, default_value = "current")]
        temporal: String,
    },
    /// Check for contradictions
    Check {
        slug: Option<String>,
        #[arg(long)]
        all: bool,
        #[arg(long)]
        r#type: Option<String>,
    },
    /// List unresolved knowledge gaps
    Gaps {
        #[arg(long, default_value = "20")]
        limit: u32,
        #[arg(long)]
        resolved: bool,
    },
    /// Checkpoint WAL to single file
    Compact,
    /// Get or set config values
    Config {
        #[command(subcommand)]
        action: commands::config::ConfigAction,
    },
    /// Validate brain integrity
    Validate {
        #[arg(long)]
        all: bool,
    },
    /// Start MCP stdio server
    Serve,
    /// Brain statistics
    Stats,
    /// Skills management
    Skills {
        #[command(subcommand)]
        action: commands::skills::SkillsAction,
    },
    /// Call a raw MCP tool
    Call { tool: String, params: Option<String> },
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();
    let db_path = cli.db.unwrap_or_else(default_db_path);
    let db = core::db::open(&db_path)?;

    match cli.command {
        Commands::Init => commands::init::run(&db),
        Commands::Get { slug } => commands::get::run(&db, &slug, cli.json),
        Commands::Put { slug } => commands::put::run(&db, &slug),
        Commands::List { wing, r#type, limit } => commands::list::run(&db, wing, r#type, limit, cli.json),
        Commands::Search { query, wing, limit } => commands::search::run(&db, &query, wing, limit, cli.json),
        Commands::Query { query, depth, token_budget, wing } => {
            commands::query::run(&db, &query, &depth, token_budget, wing, cli.json).await
        }
        Commands::Ingest { path, force } => commands::ingest::run(&db, &path, force),
        Commands::Import { path, validate_only } => commands::import::run(&db, &path, validate_only),
        Commands::Export { path, raw, import_id } => commands::export::run(&db, &path, raw, import_id),
        Commands::Embed { all, stale } => commands::embed::run(&db, all, stale),
        Commands::Link { from, to, relationship, valid_from, valid_until } => {
            commands::link::run(&db, &from, &to, &relationship, valid_from, valid_until)
        }
        Commands::Unlink { link_id } => commands::link::unlink(&db, link_id),
        Commands::Backlinks { slug, temporal } => commands::link::backlinks(&db, &slug, temporal, cli.json),
        Commands::Tag { slug, tags } => commands::tags::tag(&db, &slug, &tags),
        Commands::Untag { slug, tags } => commands::tags::untag(&db, &slug, &tags),
        Commands::Timeline { slug, limit } => commands::timeline::run(&db, &slug, limit, cli.json),
        Commands::Graph { slug, depth, temporal } => commands::graph::run(&db, &slug, depth, &temporal, cli.json),
        Commands::Check { slug, all, r#type } => commands::check::run(&db, slug, all, r#type, cli.json),
        Commands::Gaps { limit, resolved } => commands::gaps::run(&db, limit, resolved, cli.json),
        Commands::Compact => commands::compact::run(&db),
        Commands::Config { action } => commands::config::run(&db, action),
        Commands::Validate { all } => commands::validate::run(&db, all),
        Commands::Serve => commands::serve::run(&db).await,
        Commands::Stats => commands::stats::run(&db, cli.json),
        Commands::Skills { action } => commands::skills::run(action),
        Commands::Call { tool, params } => commands::call::run(&db, &tool, params).await,
    }
}
