use anyhow::Result;
use clap::{Parser, Subcommand};

mod commands;
mod core;
mod mcp;

#[derive(Parser)]
#[command(
    name = "quaid",
    version,
    about = "Local-first personal memory — SQLite + FTS5 + local vector embeddings"
)]
struct Cli {
    /// Path to memory database file [env: QUAID_DB] [default: ~/.quaid/memory.db]
    #[arg(long, env = "QUAID_DB", global = true)]
    db: Option<String>,

    /// Output as JSON
    #[arg(long, global = true)]
    json: bool,

    /// Embedding model alias or Hugging Face model ID
    #[arg(long, env = "QUAID_MODEL", global = true, default_value = "small")]
    model: String,

    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Initialise a new memory database
    Init {
        /// Path to create the new memory database
        path: Option<String>,
    },
    /// Read a page by slug
    Get { slug: String },
    /// Write or update a page (reads from stdin)
    Put {
        slug: String,
        /// Expected current version for OCC (required for Unix updates; optional for creates)
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
    /// Full-text search (sanitizes natural-language input by default; use --raw for expert FTS5 syntax)
    Search {
        query: String,
        #[arg(long)]
        wing: Option<String>,
        #[arg(long, default_value = "10")]
        limit: u32,
        /// Pass the query verbatim to FTS5 without sanitization (for expert FTS5 syntax: quoted phrases, boolean operators, wildcards)
        #[arg(long, default_value_t = false)]
        raw: bool,
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
    /// Import a markdown directory (infers page types from PARA folder structure)
    Import {
        path: String,
        #[arg(long)]
        validate_only: bool,
    },
    /// Export memory to markdown directory
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
    /// Check for contradictions using assertions from frontmatter or ## Assertions sections
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
    /// Manage vault collections
    Collection {
        #[command(subcommand)]
        action: commands::collection::CollectionAction,
    },
    /// Get or set config values
    Config {
        #[command(subcommand)]
        action: commands::config::ConfigAction,
    },
    /// Validate memory integrity
    Validate {
        /// Run all checks (default if no specific flag given)
        #[arg(long)]
        all: bool,
        /// Check link integrity
        #[arg(long)]
        links: bool,
        /// Check assertion integrity
        #[arg(long)]
        assertions: bool,
        /// Check embedding integrity
        #[arg(long)]
        embeddings: bool,
    },
    /// Start MCP stdio server
    Serve,
    /// Memory statistics
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

#[derive(Debug, PartialEq, Eq)]
enum EarlyCommand {
    None,
    Init(String),
    Version,
}

fn early_command(cli: &Cli) -> EarlyCommand {
    match &cli.command {
        Commands::Version => EarlyCommand::Version,
        Commands::Init { path } => {
            let default_path = core::db::default_db_path_string();
            let db_path = cli.db.as_deref().unwrap_or(default_path.as_str());
            EarlyCommand::Init(path.clone().unwrap_or_else(|| db_path.to_owned()))
        }
        _ => EarlyCommand::None,
    }
}

fn validate_flags_from_args(
    all: bool,
    links: bool,
    assertions: bool,
    embeddings: bool,
) -> commands::validate::CheckFlags {
    if all || (!links && !assertions && !embeddings) {
        commands::validate::CheckFlags::all()
    } else {
        commands::validate::CheckFlags {
            links,
            assertions,
            embeddings,
        }
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();
    let requested_model =
        core::inference::coerce_model_for_build(&core::inference::resolve_model(&cli.model));

    // Commands that don't require a database connection
    match early_command(&cli) {
        EarlyCommand::Version => return commands::version::run(),
        EarlyCommand::Init(path) => return commands::init::run(&path, &requested_model),
        EarlyCommand::None => {}
    }

    let db_path = cli.db.unwrap_or_else(core::db::default_db_path_string);
    let opened = core::db::open_with_model(&db_path, &requested_model)?;
    core::inference::set_model_config(opened.effective_model.clone());
    let db = opened.conn;

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
        Commands::Search {
            query,
            wing,
            limit,
            raw,
        } => commands::search::run(&db, &query, wing, limit, cli.json, raw),
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
        Commands::Collection { action } => commands::collection::run(&db, action, cli.json),
        Commands::Config { action } => commands::config::run(&db, action),
        Commands::Validate {
            all,
            links,
            assertions,
            embeddings,
        } => {
            let flags = validate_flags_from_args(all, links, assertions, embeddings);
            commands::validate::run(&db, &flags, cli.json)
        }
        Commands::Serve => commands::serve::run(db).await,
        Commands::Stats => commands::stats::run(&db, cli.json),
        Commands::Skills { action } => commands::skills::run(action, cli.json),
        Commands::Call { tool, params } => commands::call::run(db, &tool, params),
        Commands::Pipe => commands::pipe::run(db),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn early_command_returns_version_for_version_subcommand() {
        let cli = Cli::try_parse_from(["quaid", "version"]).expect("parse version");

        assert_eq!(early_command(&cli), EarlyCommand::Version);
    }

    #[test]
    fn early_command_prefers_init_path_over_global_db_flag() {
        let cli = Cli::try_parse_from(["quaid", "--db", "global.db", "init", "custom.db"])
            .expect("parse init");

        assert_eq!(
            early_command(&cli),
            EarlyCommand::Init("custom.db".to_owned())
        );
    }

    #[test]
    fn early_command_uses_global_db_flag_when_init_path_is_omitted() {
        let cli = Cli::try_parse_from(["quaid", "--db", "global.db", "init"])
            .expect("parse init without path");

        assert_eq!(
            early_command(&cli),
            EarlyCommand::Init("global.db".to_owned())
        );
    }

    #[test]
    fn validate_flags_from_args_defaults_to_all_checks() {
        let flags = validate_flags_from_args(false, false, false, false);

        assert!(flags.links);
        assert!(flags.assertions);
        assert!(flags.embeddings);
    }

    #[test]
    fn validate_flags_from_args_preserves_explicit_selection() {
        let flags = validate_flags_from_args(false, true, false, true);

        assert!(flags.links);
        assert!(!flags.assertions);
        assert!(flags.embeddings);
    }
}
