use anyhow::Result;
use clap::{Parser, Subcommand};

mod commands;
mod core;
mod mcp;

#[derive(Parser)]
#[command(
    name = "gbrain",
    version,
    about = "Personal knowledge brain — SQLite + FTS5 + local vector embeddings"
)]
struct Cli {
    /// Path to brain database file [env: GBRAIN_DB] [default: ./brain.db]
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
    Init {
        /// Path to create the new brain database
        path: Option<String>,
    },
    /// Read a page by slug
    Get { slug: String },
    /// Write or update a page (reads from stdin)
    Put {
        slug: String,
        /// Expected current version for OCC (omit for new pages or unconditional upsert)
        #[arg(long)]
        expected_version: Option<i64>,
    },
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
        /// Retrieval depth (Phase 2: progressive expansion; deferred in Phase 1)
        #[arg(long, default_value = "auto")]
        depth: String,
        #[arg(long, default_value = "10")]
        limit: u32,
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
        /// Embed a single page by slug
        slug: Option<String>,
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
    /// Close a temporal link interval by ID
    #[command(name = "link-close")]
    LinkClose {
        link_id: u64,
        #[arg(long)]
        valid_until: String,
    },
    /// List all outbound links for a page (with IDs)
    Links {
        slug: String,
        #[arg(long)]
        temporal: Option<String>,
    },
    /// Remove a cross-reference entirely
    Unlink {
        from: String,
        to: String,
        #[arg(long)]
        relationship: Option<String>,
    },
    /// List backlinks for a page
    Backlinks {
        slug: String,
        #[arg(long)]
        temporal: Option<String>,
    },
    /// Manage tags on a page (list, add, remove)
    Tags {
        slug: String,
        /// Add a tag (repeatable)
        #[arg(long)]
        add: Vec<String>,
        /// Remove a tag (repeatable)
        #[arg(long)]
        remove: Vec<String>,
    },
    /// Show timeline entries for a page
    Timeline {
        slug: String,
        #[arg(long, default_value = "20")]
        limit: u32,
    },
    /// Add a structured timeline entry
    #[command(name = "timeline-add")]
    TimelineAdd {
        slug: String,
        #[arg(long)]
        date: String,
        #[arg(long)]
        summary: String,
        #[arg(long)]
        source: Option<String>,
        #[arg(long)]
        detail: Option<String>,
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
    Call {
        tool: String,
        params: Option<String>,
    },
    /// JSONL pipe mode (one JSON object per line)
    Pipe,
    /// Print version information
    Version,
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();

    // Commands that don't require a database connection
    match &cli.command {
        Commands::Version => return commands::version::run(),
        Commands::Init { path } => {
            let db_path = cli.db.as_deref().unwrap_or("brain.db");
            let init_path = path.as_deref().unwrap_or(db_path);
            return commands::init::run(init_path);
        }
        _ => {}
    }

    let db_path = cli.db.unwrap_or_else(|| "brain.db".to_owned());
    let db = core::db::open(&db_path)?;

    match cli.command {
        Commands::Init { .. } | Commands::Version => unreachable!(),
        Commands::Get { slug } => commands::get::run(&db, &slug, cli.json),
        Commands::Put {
            slug,
            expected_version,
        } => commands::put::run(&db, &slug, expected_version),
        Commands::List {
            wing,
            r#type,
            limit,
        } => commands::list::run(&db, wing, r#type, limit, cli.json),
        Commands::Search { query, wing, limit } => {
            commands::search::run(&db, &query, wing, limit, cli.json)
        }
        Commands::Query {
            query,
            depth,
            limit,
            token_budget,
            wing,
        } => commands::query::run(&db, &query, &depth, limit, token_budget, wing, cli.json).await,
        Commands::Ingest { path, force } => commands::ingest::run(&db, &path, force),
        Commands::Import {
            path,
            validate_only,
        } => commands::import::run(&db, &path, validate_only),
        Commands::Export {
            path,
            raw,
            import_id,
        } => commands::export::run(&db, &path, raw, import_id),
        Commands::Embed { slug, all, stale } => commands::embed::run(&db, slug, all, stale),
        Commands::Link {
            from,
            to,
            relationship,
            valid_from,
            valid_until,
        } => commands::link::run(&db, &from, &to, &relationship, valid_from, valid_until),
        Commands::LinkClose {
            link_id,
            valid_until,
        } => commands::link::close(&db, link_id, &valid_until),
        Commands::Links { slug, temporal } => commands::link::links(&db, &slug, temporal, cli.json),
        Commands::Unlink {
            from,
            to,
            relationship,
        } => commands::link::unlink(&db, &from, &to, relationship),
        Commands::Backlinks { slug, temporal } => {
            commands::link::backlinks(&db, &slug, temporal, cli.json)
        }
        Commands::Tags { slug, add, remove } => commands::tags::run(&db, &slug, &add, &remove),
        Commands::Timeline { slug, limit } => commands::timeline::run(&db, &slug, limit, cli.json),
        Commands::TimelineAdd {
            slug,
            date,
            summary,
            source,
            detail,
        } => commands::timeline::add(&db, &slug, &date, &summary, source, detail),
        Commands::Graph {
            slug,
            depth,
            temporal,
        } => commands::graph::run(&db, &slug, depth, &temporal, cli.json),
        Commands::Check { slug, all, r#type } => {
            commands::check::run(&db, slug, all, r#type, cli.json)
        }
        Commands::Gaps { limit, resolved } => commands::gaps::run(&db, limit, resolved, cli.json),
        Commands::Compact => commands::compact::run(&db),
        Commands::Config { action } => commands::config::run(&db, action),
        Commands::Validate { all } => commands::validate::run(&db, all),
        Commands::Serve => commands::serve::run(&db).await,
        Commands::Stats => commands::stats::run(&db, cli.json),
        Commands::Skills { action } => commands::skills::run(action),
        Commands::Call { tool, params } => commands::call::run(&db, &tool, params).await,
        Commands::Pipe => commands::pipe::run(&db),
    }
}
