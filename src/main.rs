#![allow(
    clippy::print_stdout,
    reason = "the quaid CLI prints user-facing output to stdout by design; print_stdout is appropriate for production code in lib/CLI commands but not the CLI binary itself"
)]
#![cfg_attr(
    test,
    allow(
        clippy::expect_used,
        clippy::panic,
        reason = "test fixtures legitimately panic/expect on setup failure; the lib-crate's test exemption doesn't reach the bin crate"
    )
)]

use anyhow::Result;
use clap::{Parser, Subcommand};

use quaid::{commands, core};

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
    Get {
        slug: String,
        #[arg(long)]
        namespace: Option<String>,
    },
    /// Write or update a page (reads from stdin)
    Put {
        slug: String,
        #[arg(long)]
        namespace: Option<String>,
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
        #[arg(long)]
        namespace: Option<String>,
        #[arg(long, default_value = "50")]
        limit: u32,
    },
    /// Full-text search (sanitizes natural-language input by default; use --raw for expert FTS5 syntax)
    Search {
        query: String,
        #[arg(long)]
        wing: Option<String>,
        #[arg(long)]
        namespace: Option<String>,
        #[arg(long, default_value = "10")]
        limit: u32,
        #[arg(long, default_value_t = false)]
        include_superseded: bool,
        /// Pass the query verbatim to FTS5 without sanitization (for expert FTS5 syntax: quoted phrases, boolean operators, wildcards)
        #[arg(long, default_value_t = false)]
        raw: bool,
        /// Override `config.graph_depth` for this invocation. `0` disables graph expansion.
        #[arg(long)]
        hops: Option<u32>,
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
        #[arg(long)]
        namespace: Option<String>,
        #[arg(long, default_value_t = false)]
        include_superseded: bool,
        /// Override `config.graph_depth` for this invocation. `0` disables graph expansion.
        #[arg(long)]
        hops: Option<u32>,
    },
    /// Ingest a source document
    Ingest {
        path: String,
        #[arg(long)]
        force: bool,
    },
    /// Export memory to markdown directory
    Export {
        path: String,
        #[arg(long)]
        raw: bool,
        #[arg(long)]
        import_id: Option<String>,
    },
    /// Control conversation extraction runtime state
    Extraction {
        #[command(subcommand)]
        action: commands::extraction::ExtractionAction,
    },
    /// Re-enqueue manual extraction for one or more sessions
    Extract(commands::extract::ExtractArgs),
    /// Manage cached local models
    Model {
        #[command(subcommand)]
        action: commands::model::ModelAction,
    },
    /// Generate or refresh embeddings
    Embed {
        /// Embed a single page by slug
        slug: Option<String>,
        #[arg(long)]
        all: bool,
        #[arg(long)]
        stale: bool,
        /// Maximum number of pages to scan per batch for bulk embedding
        #[arg(long)]
        batch_size: Option<usize>,
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
    Graph(commands::graph::GraphArgs),
    /// Check for contradictions across assertions (frontmatter allowlist,
    /// ## Assertions sections, extracted facts), or resolve one by id
    Check {
        slug: Option<String>,
        #[arg(long)]
        all: bool,
        #[arg(long)]
        r#type: Option<String>,
        /// Mark a contradiction resolved by id (stamps resolved_at)
        #[arg(long, conflicts_with_all = ["slug", "all", "type"])]
        resolve: Option<i64>,
        /// Slug of the page to keep when resolving; the other page in the
        /// contradiction is superseded by it
        #[arg(long, requires = "resolve")]
        keep: Option<String>,
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
    /// Manage memory namespaces
    Namespace {
        #[command(subcommand)]
        action: commands::namespace::NamespaceAction,
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
    /// Start MCP server. Defaults to stdio; `--http` opens the SSE
    /// transport on loopback instead.
    Serve {
        /// Open the HTTP/SSE MCP transport (mutually exclusive with stdio,
        /// which is the default).
        #[arg(long)]
        http: bool,
        /// TCP port for the HTTP transport (default: 3112). Requires `--http`.
        #[arg(long, requires = "http")]
        port: Option<u16>,
        /// Bind address for the HTTP transport (default: 127.0.0.1).
        /// Non-loopback binds are refused in v1; requires `--http`.
        #[arg(long, requires = "http")]
        bind: Option<std::net::IpAddr>,
        /// Path to a bearer-token file. Parsed and validated but not
        /// enforced in v1 (see HTTP transport docs); requires `--http`.
        #[arg(long, requires = "http")]
        token_file: Option<std::path::PathBuf>,
        /// Treat the loopback interface as trusted and allow unauthenticated
        /// access (matches stdio's security profile). Requires `--http`.
        #[arg(long, requires = "http")]
        trust_loopback: bool,
    },
    /// Background-daemon lifecycle: install, run, status, logs, etc.
    Daemon {
        #[command(subcommand)]
        action: commands::daemon::DaemonAction,
    },
    /// Process-level status: daemon installed/running, MCP transports,
    /// recent runtime activity. Distinct from `quaid stats` (content-level).
    Status {
        /// Emit machine-readable JSON.
        #[arg(long)]
        json: bool,
    },
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
    Model(commands::model::ModelAction),
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
        Commands::Model { action } => EarlyCommand::Model(action.clone()),
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
        EarlyCommand::Model(action) => return commands::model::run(action),
        EarlyCommand::None => {}
    }

    let db_path = cli.db.unwrap_or_else(core::db::default_db_path_string);
    let opened = core::db::open_with_model(&db_path, &requested_model)?;
    core::inference::set_model_config(opened.effective_model.clone());
    let db = opened.conn;

    match cli.command {
        Commands::Init { .. } | Commands::Model { .. } | Commands::Version => unreachable!(),
        Commands::Get { slug, namespace } => {
            commands::get::run(&db, &slug, namespace.as_deref().or(Some("")), cli.json)
        }
        Commands::Put {
            slug,
            namespace,
            expected_version,
        } => commands::put::run(&db, &slug, namespace.as_deref(), expected_version),
        Commands::List {
            wing,
            r#type,
            namespace,
            limit,
        } => commands::list::run(
            &db,
            wing,
            r#type,
            namespace.as_deref().or(Some("")),
            limit,
            cli.json,
        ),
        Commands::Search {
            query,
            wing,
            namespace,
            limit,
            include_superseded,
            raw,
            hops,
        } => commands::search::run(
            &db,
            &query,
            wing,
            namespace.as_deref().or(Some("")),
            limit,
            include_superseded,
            cli.json,
            raw,
            hops,
        ),
        Commands::Query {
            query,
            depth,
            limit,
            token_budget,
            wing,
            namespace,
            include_superseded,
            hops,
        } => {
            commands::query::run(
                &db,
                &query,
                &depth,
                limit,
                token_budget,
                wing,
                namespace.as_deref().or(Some("")),
                include_superseded,
                cli.json,
                hops,
            )
            .await
        }
        Commands::Ingest { path, force } => commands::ingest::run(&db, &path, force),
        Commands::Export {
            path,
            raw,
            import_id,
        } => commands::export::run(&db, &path, raw, import_id),
        Commands::Extraction { action } => commands::extraction::run(&db, action),
        Commands::Extract(args) => commands::extract::run(&db, args),
        Commands::Embed {
            slug,
            all,
            stale,
            batch_size,
        } => commands::embed::run_with_batch(&db, slug, all, stale, batch_size),
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
        Commands::Graph(args) => commands::graph::run_cli(&db, args, cli.json),
        Commands::Check {
            slug,
            all,
            r#type,
            resolve,
            keep,
        } => match resolve {
            Some(contradiction_id) => {
                commands::check::run_resolve(&db, contradiction_id, keep.as_deref(), cli.json)
            }
            None => commands::check::run(&db, slug, all, r#type, cli.json),
        },
        Commands::Gaps { limit, resolved } => commands::gaps::run(&db, limit, resolved, cli.json),
        Commands::Compact => commands::compact::run(&db),
        Commands::Collection { action } => commands::collection::run(&db, action, cli.json),
        Commands::Namespace { action } => commands::namespace::run(&db, action, cli.json),
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
        Commands::Serve {
            http,
            port,
            bind,
            token_file,
            trust_loopback,
        } => {
            let http_config = if http {
                Some(quaid::mcp::HttpConfig {
                    port: port.unwrap_or(quaid::mcp::http::DEFAULT_HTTP_PORT),
                    bind: bind.unwrap_or(quaid::mcp::http::DEFAULT_HTTP_BIND),
                    token_file,
                    trusted_loopback: trust_loopback,
                })
            } else {
                None
            };
            commands::serve::run(db, http_config).await
        }
        Commands::Daemon { action } => {
            let code = commands::daemon::run(action, db).await?;
            if code != 0 {
                std::process::exit(i32::from(code));
            }
            Ok(())
        }
        Commands::Status { json } => {
            let code = commands::status::run(&db, json)?;
            if code != 0 {
                std::process::exit(i32::from(code));
            }
            Ok(())
        }
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
    fn early_command_returns_none_for_regular_subcommands() {
        let cli = Cli::try_parse_from(["quaid", "config", "list"]).expect("parse config list");

        assert_eq!(early_command(&cli), EarlyCommand::None);
    }

    #[test]
    fn early_command_treats_model_pull_as_database_free() {
        let cli = Cli::try_parse_from(["quaid", "model", "pull", "phi-3.5-mini"])
            .expect("parse model pull");

        match early_command(&cli) {
            EarlyCommand::Model(commands::model::ModelAction::Pull { alias }) => {
                assert_eq!(alias, "phi-3.5-mini");
            }
            other => panic!("expected early model command, got {other:?}"),
        }
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

    #[test]
    fn validate_flags_from_args_honors_all_flag() {
        let flags = validate_flags_from_args(true, false, false, false);

        assert!(flags.links);
        assert!(flags.assertions);
        assert!(flags.embeddings);
    }
}
