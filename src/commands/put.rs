#![expect(
    clippy::print_stdout,
    reason = "CLI command prints user-facing output to stdout by design"
)]

use std::io::{self, Read};

#[cfg(unix)]
use std::fs::{self, File};
#[cfg(unix)]
use std::io::{BufRead, BufReader, Write};
#[cfg(unix)]
use std::os::unix::fs::{FileTypeExt, MetadataExt};
#[cfg(unix)]
use std::os::unix::net::UnixStream;
#[cfg(unix)]
use std::path::{Path, PathBuf};
#[cfg(unix)]
use std::time::Duration;

use rusqlite::Connection;
#[cfg(unix)]
use rustix::{fd::AsFd, fs::fsync};
use serde_json::Value as JsonValue;
use sha2::{Digest, Sha256};
#[cfg(unix)]
use uuid::Uuid;

#[cfg(unix)]
use crate::core::fs_safety;
use crate::core::supersede;
use crate::core::types::{frontmatter_get_string, Frontmatter};
use crate::core::{
    entities, file_state, links, markdown, page_uuid, palace, raw_imports, vault_sync,
};

#[derive(Debug, Clone)]
struct PreparedPut {
    collection_id: i64,
    collection_name: String,
    namespace: String,
    slug: String,
    page_uuid: String,
    page_type: String,
    title: String,
    summary: String,
    compiled_truth: String,
    timeline: String,
    frontmatter: Frontmatter,
    frontmatter_json: String,
    supersedes: Option<String>,
    wing: String,
    room: String,
    now: String,
    current_version: Option<i64>,
    sha256: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct PutOutcome {
    created: bool,
    version: i64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct StagedPageRecord {
    page_id: i64,
    outcome: PutOutcome,
}

#[cfg(unix)]
#[cfg_attr(not(test), allow(dead_code))]
#[derive(Debug, Clone, Default)]
struct PutTestHooks {
    fail_sentinel_create: bool,
    fail_before_rename: bool,
    fail_rename: bool,
    fail_commit: bool,
    fail_parent_fsync: bool,
    block_inside_slug_lock: bool,
    block_after_supersede_claim: bool,
    skip_dirty_mark: bool,
    post_rename_swap: Option<Vec<u8>>,
}

#[cfg(unix)]
impl PutTestHooks {
    fn before_rename(&self, _target_path: &Path) -> Result<(), vault_sync::VaultSyncError> {
        if self.fail_before_rename {
            Err(io::Error::other("injected pre-rename failure").into())
        } else {
            Ok(())
        }
    }

    fn after_rename(
        &self,
        _db: &Connection,
        target_path: &Path,
    ) -> Result<(), vault_sync::VaultSyncError> {
        if let Some(replacement) = self.post_rename_swap.as_ref() {
            let foreign_temp = target_path.with_file_name(format!(".foreign-{}", Uuid::now_v7()));
            fs::write(&foreign_temp, replacement)?;
            fs::rename(&foreign_temp, target_path)?;
        }
        Ok(())
    }
}

/// Read markdown from stdin, parse it, and insert or update a page.
///
/// OCC contract:
/// - New page (no row for `slug`): INSERT with `version = 1`.
/// - Existing page + `--expected-version N`: compare-and-swap UPDATE.
///   If stored version ≠ N → print conflict with current version, exit 1.
/// - On Unix vault write-through paths, existing pages MUST provide
///   `--expected-version`; omission fails closed before sentinel creation.
/// - Non-Unix paths fail closed with `UnsupportedPlatformError`.
pub fn run(
    db: &Connection,
    slug: &str,
    namespace: Option<&str>,
    expected_version: Option<i64>,
    json: bool,
) -> anyhow::Result<()> {
    vault_sync::ensure_unix_platform("quaid put").map_err(anyhow::Error::new)?;
    let mut input = String::new();
    io::stdin().read_to_string(&mut input)?;
    let status = put_from_cli_string(db, slug, &input, namespace, expected_version)?;
    if json {
        println!("{}", serde_json::json!({ "result": status }));
    } else {
        println!("{status}");
    }
    Ok(())
}

#[cfg(unix)]
fn put_from_cli_string(
    db: &Connection,
    slug_input: &str,
    content: &str,
    namespace: Option<&str>,
    expected_version: Option<i64>,
) -> anyhow::Result<String> {
    crate::core::namespace::validate_optional_namespace(namespace)?;
    let op_kind = if expected_version.is_some() {
        crate::core::collections::OpKind::WriteUpdate
    } else {
        crate::core::collections::OpKind::WriteCreate
    };
    let resolved =
        vault_sync::resolve_slug_for_op(db, slug_input, op_kind).map_err(anyhow::Error::new)?;
    let collection = vault_sync::load_collection_by_id(db, resolved.collection_id)
        .map_err(anyhow::Error::new)?;
    let canonical_slug = format!("{}::{}", resolved.collection_name, resolved.slug);
    match vault_sync::live_serve_endpoint_for_root_path(db, &collection.root_path) {
        Ok(Some(endpoint)) => {
            return proxy_put_via_live_serve(&endpoint, &canonical_slug, content, expected_version);
        }
        Ok(None) => {}
        Err(other) => return Err(anyhow::Error::new(other)),
    }
    let _lease = vault_sync::start_short_lived_owner_lease_for_root_path(db, &collection.root_path)
        .map_err(anyhow::Error::new)?;
    let status =
        put_from_string_status_with_namespace(db, &canonical_slug, content, namespace, expected_version)?;
    // Drain the embedding queue inline so CLI-only users (no running daemon)
    // get semantic results for the page they just wrote. Job claiming is
    // transactional, so this is safe even if a daemon races us (review #10).
    match vault_sync::drain_embedding_queue(db) {
        Ok(drained) if drained > 0 => {
            println!("Embedded {drained} pending page(s)");
        }
        Ok(_) => {}
        Err(error) => {
            eprintln!("WARN: cli_put_embedding_drain_failed error={error}");
        }
    }
    Ok(status)
}

#[cfg(not(unix))]
fn put_from_cli_string(
    db: &Connection,
    slug_input: &str,
    content: &str,
    namespace: Option<&str>,
    expected_version: Option<i64>,
) -> anyhow::Result<String> {
    put_from_string_status_with_namespace(db, slug_input, content, namespace, expected_version)
}

/// Apply page content supplied by the caller.
pub fn put_from_string(
    db: &Connection,
    slug_input: &str,
    content: &str,
    expected_version: Option<i64>,
) -> anyhow::Result<()> {
    put_from_string_with_namespace(db, slug_input, content, None, expected_version)
}

/// Apply page content supplied by the caller into a namespace.
pub fn put_from_string_with_namespace(
    db: &Connection,
    slug_input: &str,
    content: &str,
    namespace: Option<&str>,
    expected_version: Option<i64>,
) -> anyhow::Result<()> {
    let status = put_from_string_status_with_namespace(
        db,
        slug_input,
        content,
        namespace,
        expected_version,
    )?;
    println!("{status}");
    Ok(())
}

pub(crate) fn put_from_string_quiet(
    db: &Connection,
    slug_input: &str,
    content: &str,
    expected_version: Option<i64>,
) -> anyhow::Result<()> {
    put_from_string_status(db, slug_input, content, expected_version).map(|_| ())
}

pub(crate) fn put_from_string_quiet_with_namespace(
    db: &Connection,
    slug_input: &str,
    content: &str,
    namespace: Option<&str>,
    expected_version: Option<i64>,
) -> anyhow::Result<()> {
    put_from_string_status_with_namespace(db, slug_input, content, namespace, expected_version)
        .map(|_| ())
}

pub(crate) fn put_from_string_status(
    db: &Connection,
    slug_input: &str,
    content: &str,
    expected_version: Option<i64>,
) -> anyhow::Result<String> {
    put_from_string_with_output(db, slug_input, content, None, expected_version)
}

pub(crate) fn put_from_string_status_with_namespace(
    db: &Connection,
    slug_input: &str,
    content: &str,
    namespace: Option<&str>,
    expected_version: Option<i64>,
) -> anyhow::Result<String> {
    put_from_string_with_output(db, slug_input, content, namespace, expected_version)
}

fn put_from_string_with_output(
    db: &Connection,
    slug_input: &str,
    content: &str,
    namespace: Option<&str>,
    expected_version: Option<i64>,
) -> anyhow::Result<String> {
    crate::core::namespace::validate_optional_namespace(namespace)?;
    let namespace = namespace.unwrap_or("").to_owned();
    let (frontmatter, body) = markdown::parse_frontmatter(content);
    let (compiled_truth, timeline) = markdown::split_content(&body);
    let summary = markdown::extract_summary(&compiled_truth);
    links::validate_graph_frontmatter(&frontmatter)
        .map_err(|err| anyhow::anyhow!("malformed frontmatter graph input: {err}"))?;
    // Entity-pattern validation must fail BEFORE any page mutation
    // (task 7.6 — malformed YAML/regex/capture/weight fails closed).
    let _entity_patterns_validated = entities::load_patterns(db)
        .map_err(|err| anyhow::anyhow!("entity pattern load failed: {err}"))?;
    let op_kind = if expected_version.is_some() {
        crate::core::collections::OpKind::WriteUpdate
    } else {
        crate::core::collections::OpKind::WriteCreate
    };
    let resolved =
        vault_sync::resolve_slug_for_op(db, slug_input, op_kind).map_err(anyhow::Error::new)?;
    vault_sync::ensure_collection_vault_write_allowed(db, resolved.collection_id)
        .map_err(anyhow::Error::new)?;
    let collection = vault_sync::load_collection_by_id(db, resolved.collection_id)
        .map_err(anyhow::Error::new)?;
    let slug = resolved.slug.as_str();
    let wing = palace::derive_wing(slug);
    let room = palace::derive_room(&compiled_truth);

    let title = frontmatter_get_string(&frontmatter, "title").unwrap_or_else(|| slug.to_string());
    let page_type =
        frontmatter_get_string(&frontmatter, "type").unwrap_or_else(|| "concept".to_string());

    let relative_path = slug_to_relative_path(slug);
    let now = now_iso_from(db);
    let (prepared, outcome) = vault_sync::with_write_slug_lock(
        &collection.root_path,
        &relative_path,
        || -> anyhow::Result<(PreparedPut, PutOutcome)> {
            maybe_block_inside_write_lock(db);
            let existing_row: Option<(i64, i64, Option<String>)> = match db
                .prepare(
                    "SELECT id, version, uuid
                     FROM pages
                     WHERE collection_id = ?1 AND namespace = ?2 AND slug = ?3",
                )?
                .query_row(
                    rusqlite::params![resolved.collection_id, namespace, slug],
                    |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
                ) {
                Ok(v) => Some(v),
                Err(rusqlite::Error::QueryReturnedNoRows) => None,
                Err(e) => return Err(e.into()),
            };
            let current_page_id = existing_row.as_ref().map(|(page_id, _, _)| *page_id);
            let page_uuid = page_uuid::resolve_page_uuid(
                &frontmatter,
                existing_row
                    .as_ref()
                    .and_then(|(_, _, uuid)| uuid.as_deref()),
            )?;
            let supersedes = frontmatter
                .get("supersedes")
                .and_then(JsonValue::as_str)
                .map(str::to_owned);
            supersede::validate_supersede_target(
                db,
                resolved.collection_id,
                &namespace,
                current_page_id,
                slug,
                supersedes.as_deref(),
            )?;
            let prepared = PreparedPut {
                collection_id: resolved.collection_id,
                collection_name: resolved.collection_name.clone(),
                namespace: namespace.clone(),
                slug: slug.to_owned(),
                page_uuid,
                page_type,
                title: title.clone(),
                summary: summary.clone(),
                compiled_truth: compiled_truth.clone(),
                timeline: timeline.clone(),
                frontmatter: frontmatter.clone(),
                frontmatter_json: serde_json::to_string(&frontmatter)?,
                supersedes,
                wing: wing.clone(),
                room: room.clone(),
                now: now.clone(),
                current_version: existing_row.map(|(_, version, _)| version),
                sha256: sha256_hex(content.as_bytes()),
            };
            let outcome = persist_with_vault_write(
                db,
                &prepared,
                content.as_bytes(),
                &relative_path,
                expected_version,
            )
            .map_err(anyhow::Error::new)?;
            Ok((prepared, outcome))
        },
    )
    .map_err(anyhow::Error::new)??;

    let verb = if outcome.created {
        "Created"
    } else {
        "Updated"
    };
    Ok(format!(
        "{verb} {}::{} (version {})",
        prepared.collection_name, prepared.slug, outcome.version
    ))
}

#[cfg(unix)]
fn proxy_put_via_live_serve(
    endpoint: &vault_sync::LiveServeEndpoint,
    slug: &str,
    content: &str,
    expected_version: Option<i64>,
) -> anyhow::Result<String> {
    let socket_path = Path::new(&endpoint.ipc_path);
    let metadata = fs::symlink_metadata(socket_path).map_err(|error| {
        anyhow::Error::new(vault_sync::VaultSyncError::Ipc(
            vault_sync::IpcError::IpcPeerAuthFailed {
                path: endpoint.ipc_path.clone(),
                reason: error.to_string(),
            },
        ))
    })?;
    let mode = metadata.mode() & 0o777;
    if !metadata.file_type().is_socket() {
        return Err(anyhow::Error::new(vault_sync::VaultSyncError::Ipc(
            vault_sync::IpcError::IpcPeerAuthFailed {
                path: endpoint.ipc_path.clone(),
                reason: "path is not a unix socket".to_owned(),
            },
        )));
    }
    if metadata.uid() != vault_sync::current_effective_uid() {
        return Err(anyhow::Error::new(vault_sync::VaultSyncError::Ipc(
            vault_sync::IpcError::IpcPeerAuthFailed {
                path: endpoint.ipc_path.clone(),
                reason: format!(
                    "socket uid {} does not match current uid {}",
                    metadata.uid(),
                    vault_sync::current_effective_uid()
                ),
            },
        )));
    }
    if mode != 0o600 {
        return Err(anyhow::Error::new(vault_sync::VaultSyncError::Ipc(
            vault_sync::IpcError::IpcPeerAuthFailed {
                path: endpoint.ipc_path.clone(),
                reason: format!("socket mode {:o} is not 600", mode),
            },
        )));
    }
    let path_session_id = vault_sync::session_id_from_ipc_path(socket_path).ok_or_else(|| {
        anyhow::Error::new(vault_sync::VaultSyncError::Ipc(
            vault_sync::IpcError::IpcPeerAuthFailed {
                path: endpoint.ipc_path.clone(),
                reason: "socket path does not embed a session id".to_owned(),
            },
        ))
    })?;

    let mut stream = UnixStream::connect(socket_path).map_err(|error| {
        anyhow::Error::new(vault_sync::VaultSyncError::Ipc(
            vault_sync::IpcError::IpcPeerAuthFailed {
                path: endpoint.ipc_path.clone(),
                reason: error.to_string(),
            },
        ))
    })?;
    stream
        .set_read_timeout(Some(Duration::from_secs(5)))
        .map_err(anyhow::Error::new)?;
    stream
        .set_write_timeout(Some(Duration::from_secs(5)))
        .map_err(anyhow::Error::new)?;
    let peer = vault_sync::peer_credentials_for_stream(&stream).map_err(anyhow::Error::new)?;
    if path_session_id != endpoint.session_id {
        return Err(anyhow::Error::new(vault_sync::VaultSyncError::Ipc(
            vault_sync::IpcError::IpcPeerAuthFailed {
                path: endpoint.ipc_path.clone(),
                reason: format!(
                    "path session {} does not match owner session {}",
                    path_session_id, endpoint.session_id
                ),
            },
        )));
    }
    if peer.uid != vault_sync::current_effective_uid() {
        return Err(anyhow::Error::new(vault_sync::VaultSyncError::Ipc(
            vault_sync::IpcError::IpcPeerAuthFailed {
                path: endpoint.ipc_path.clone(),
                reason: format!(
                    "peer uid {} does not match current uid {}",
                    peer.uid,
                    vault_sync::current_effective_uid()
                ),
            },
        )));
    }
    if i64::from(peer.pid) != endpoint.pid {
        return Err(anyhow::Error::new(vault_sync::VaultSyncError::Ipc(
            vault_sync::IpcError::IpcPeerAuthFailed {
                path: endpoint.ipc_path.clone(),
                reason: format!(
                    "peer pid {} does not match owner pid {}",
                    peer.pid, endpoint.pid
                ),
            },
        )));
    }

    let read_stream = stream.try_clone()?;
    let mut reader = BufReader::new(read_stream);
    send_ipc_request(&mut stream, &vault_sync::IpcRequest::WhoAmI)?;
    let whoami = read_ipc_response(&mut reader, socket_path)?;
    let whoami_session_id = match whoami {
        vault_sync::IpcResponse::WhoAmI { session_id } => session_id,
        vault_sync::IpcResponse::Error { error } => {
            return Err(anyhow::Error::new(vault_sync::VaultSyncError::Ipc(
                vault_sync::IpcError::IpcPeerAuthFailed {
                    path: endpoint.ipc_path.clone(),
                    reason: error,
                },
            )));
        }
        other => {
            return Err(anyhow::Error::new(vault_sync::VaultSyncError::Ipc(
                vault_sync::IpcError::IpcPeerAuthFailed {
                    path: endpoint.ipc_path.clone(),
                    reason: format!("unexpected whoami response: {other:?}"),
                },
            )));
        }
    };

    vault_sync::authorize_client_peer(
        socket_path,
        &path_session_id,
        &endpoint.session_id,
        endpoint.pid,
        &peer,
        &whoami_session_id,
    )
    .map_err(anyhow::Error::new)?;

    send_ipc_request(
        &mut stream,
        &vault_sync::IpcRequest::Put {
            slug: slug.to_owned(),
            content: content.to_owned(),
            expected_version,
        },
    )?;
    match read_ipc_response(&mut reader, socket_path)? {
        vault_sync::IpcResponse::PutOk { status } => Ok(status),
        vault_sync::IpcResponse::Error { error } => Err(anyhow::anyhow!("{error}")),
        other => Err(anyhow::Error::new(vault_sync::VaultSyncError::Ipc(
            vault_sync::IpcError::IpcPeerAuthFailed {
                path: endpoint.ipc_path.clone(),
                reason: format!("unexpected put response: {other:?}"),
            },
        ))),
    }
}

#[cfg(unix)]
fn send_ipc_request(
    stream: &mut UnixStream,
    request: &vault_sync::IpcRequest,
) -> anyhow::Result<()> {
    serde_json::to_writer(&mut *stream, request)?;
    stream.write_all(b"\n")?;
    stream.flush()?;
    Ok(())
}

#[cfg(unix)]
fn read_ipc_response(
    reader: &mut BufReader<UnixStream>,
    socket_path: &Path,
) -> anyhow::Result<vault_sync::IpcResponse> {
    let mut line = String::new();
    let bytes_read = reader.read_line(&mut line)?;
    if bytes_read == 0 {
        return Err(anyhow::Error::new(vault_sync::VaultSyncError::Ipc(
            vault_sync::IpcError::IpcPeerAuthFailed {
                path: socket_path.display().to_string(),
                reason: "connection closed before response".to_owned(),
            },
        )));
    }
    serde_json::from_str(line.trim_end()).map_err(|error| {
        anyhow::Error::new(vault_sync::VaultSyncError::Ipc(
            vault_sync::IpcError::IpcPeerAuthFailed {
                path: socket_path.display().to_string(),
                reason: format!("invalid ipc response: {error}"),
            },
        ))
    })
}

/// Get current UTC timestamp in ISO 8601 format from SQLite.
/// Keeps us dependency-free (no chrono) and consistent with schema defaults.
fn now_iso_from(db: &Connection) -> String {
    db.query_row("SELECT strftime('%Y-%m-%dT%H:%M:%SZ', 'now')", [], |row| {
        row.get(0)
    })
    .unwrap_or_else(|_| "1970-01-01T00:00:00Z".to_string())
}

fn stage_page_record(
    tx: &rusqlite::Transaction<'_>,
    prepared: &PreparedPut,
    expected_version: Option<i64>,
) -> Result<StagedPageRecord, vault_sync::VaultSyncError> {
    let (created, version) = match prepared.current_version {
        None => {
            tx.execute(
                "INSERT INTO pages \
                     (collection_id, slug, uuid, type, title, summary, compiled_truth, timeline, \
                      frontmatter, wing, room, version, namespace, \
                        created_at, updated_at, truth_updated_at, timeline_updated_at) \
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, 1, ?12, ?13, ?13, ?13, ?13)",
                rusqlite::params![
                    prepared.collection_id,
                    prepared.slug,
                    prepared.page_uuid,
                    prepared.page_type,
                    prepared.title,
                    prepared.summary,
                    prepared.compiled_truth,
                    prepared.timeline,
                    prepared.frontmatter_json,
                    prepared.wing,
                    prepared.room,
                    prepared.namespace,
                    prepared.now,
                ],
            )?;
            (true, 1)
        }
        Some(current) => {
            let rows = if let Some(expected) = expected_version {
                tx.execute(
                    "UPDATE pages SET \
                         uuid = ?1, type = ?2, title = ?3, summary = ?4, \
                          compiled_truth = ?5, timeline = ?6, \
                          frontmatter = ?7, wing = ?8, room = ?9, \
                          version = version + 1, \
                          updated_at = ?10, truth_updated_at = ?10, timeline_updated_at = ?10 \
                     WHERE collection_id = ?11 AND namespace = ?12 AND slug = ?13 AND version = ?14",
                    rusqlite::params![
                        prepared.page_uuid,
                        prepared.page_type,
                        prepared.title,
                        prepared.summary,
                        prepared.compiled_truth,
                        prepared.timeline,
                        prepared.frontmatter_json,
                        prepared.wing,
                        prepared.room,
                        prepared.now,
                        prepared.collection_id,
                        prepared.namespace,
                        prepared.slug,
                        expected,
                    ],
                )?
            } else {
                tx.execute(
                    "UPDATE pages SET \
                         uuid = ?1, type = ?2, title = ?3, summary = ?4, \
                          compiled_truth = ?5, timeline = ?6, \
                          frontmatter = ?7, wing = ?8, room = ?9, \
                          version = version + 1, \
                          updated_at = ?10, truth_updated_at = ?10, timeline_updated_at = ?10 \
                     WHERE collection_id = ?11 AND namespace = ?12 AND slug = ?13",
                    rusqlite::params![
                        prepared.page_uuid,
                        prepared.page_type,
                        prepared.title,
                        prepared.summary,
                        prepared.compiled_truth,
                        prepared.timeline,
                        prepared.frontmatter_json,
                        prepared.wing,
                        prepared.room,
                        prepared.now,
                        prepared.collection_id,
                        prepared.namespace,
                        prepared.slug,
                    ],
                )?
            };

            if rows == 0 {
                return Err(crate::core::types::OccError::Conflict {
                    current_version: current,
                }
                .into());
            }

            (false, current + 1)
        }
    };

    let page_id: i64 = tx.query_row(
        "SELECT id FROM pages WHERE collection_id = ?1 AND namespace = ?2 AND slug = ?3",
        rusqlite::params![prepared.collection_id, prepared.namespace, prepared.slug],
        |row| row.get(0),
    )?;

    supersede::reconcile_supersede_chain(
        tx,
        prepared.collection_id,
        &prepared.namespace,
        page_id,
        &prepared.slug,
        prepared.supersedes.as_deref(),
    )
    .map_err(|error| rusqlite::Error::InvalidParameterName(error.to_string()))?;

    links::sync_page_graph_artifacts(
        tx,
        page_id,
        prepared.collection_id,
        &prepared.frontmatter,
        &prepared.compiled_truth,
        &prepared.timeline,
    )
    .map_err(|error| rusqlite::Error::InvalidParameterName(error.to_string()))?;

    // Entity-pattern extraction (Wave 5 / task 7.6). Patterns were validated
    // upfront in `put_from_string_with_namespace`; reloading here is a cheap
    // no-op against the embedded YAML. Failures are swallowed so they cannot
    // corrupt the page write.
    if let Ok(patterns) = entities::load_patterns(tx) {
        entities::try_run_for_page(
            tx,
            page_id,
            prepared.collection_id,
            &prepared.slug,
            &prepared.compiled_truth,
            &patterns,
        );
    }

    Ok(StagedPageRecord {
        page_id,
        outcome: PutOutcome { created, version },
    })
}

fn commit_staged_page_record(
    tx: rusqlite::Transaction<'_>,
    prepared: &PreparedPut,
    staged: StagedPageRecord,
    raw_bytes: &[u8],
    relative_path: &str,
    file_stat: Option<&file_state::FileStat>,
) -> Result<PutOutcome, rusqlite::Error> {
    supersede::reconcile_supersede_chain(
        &tx,
        prepared.collection_id,
        &prepared.namespace,
        staged.page_id,
        &prepared.slug,
        prepared.supersedes.as_deref(),
    )
    .map_err(|error| rusqlite::Error::InvalidParameterName(error.to_string()))?;

    if let Some(file_stat) = file_stat {
        file_state::upsert_file_state(
            &tx,
            prepared.collection_id,
            relative_path,
            staged.page_id,
            file_stat,
            &prepared.sha256,
        )?;
    }
    raw_imports::rotate_active_raw_import(&tx, staged.page_id, relative_path, raw_bytes)?;
    raw_imports::enqueue_embedding_job(&tx, staged.page_id)?;
    tx.commit()?;

    Ok(staged.outcome)
}

fn persist_page_record(
    db: &Connection,
    prepared: &PreparedPut,
    raw_bytes: &[u8],
    relative_path: &str,
    file_stat: Option<&file_state::FileStat>,
    expected_version: Option<i64>,
) -> Result<PutOutcome, vault_sync::VaultSyncError> {
    // BEGIN IMMEDIATE so the reserved write lock is taken at txn start and the
    // 5s busy_timeout covers cross-process contention, rather than risking a
    // transient SQLITE_BUSY from inside an already-open deferred transaction.
    // (VaultSyncError: From<rusqlite::Error> converts the begin error via `?`.)
    let tx = crate::core::db::begin_immediate(db)?;
    let staged = stage_page_record(&tx, prepared, expected_version)?;
    commit_staged_page_record(tx, prepared, staged, raw_bytes, relative_path, file_stat)
        .map_err(Into::into)
}

#[cfg(not(unix))]
fn persist_with_vault_write(
    db: &Connection,
    prepared: &PreparedPut,
    raw_bytes: &[u8],
    relative_path: &str,
    expected_version: Option<i64>,
) -> Result<PutOutcome, vault_sync::VaultSyncError> {
    persist_page_record(
        db,
        prepared,
        raw_bytes,
        relative_path,
        None,
        expected_version,
    )
}

#[cfg(unix)]
#[expect(
    clippy::question_mark,
    reason = "explicit if-let early-return makes the empty/in-memory db_path guard control flow more readable than the equivalent ? chain"
)]
fn persist_with_vault_write(
    db: &Connection,
    prepared: &PreparedPut,
    raw_bytes: &[u8],
    relative_path: &str,
    expected_version: Option<i64>,
) -> Result<PutOutcome, vault_sync::VaultSyncError> {
    let db_path = vault_sync::database_path(db)?;
    if db_path.is_empty() || db_path == ":memory:" {
        return persist_page_record(
            db,
            prepared,
            raw_bytes,
            relative_path,
            None,
            expected_version,
        );
    }

    let collection = vault_sync::load_collection_by_id(db, prepared.collection_id)?;
    let relative_path_buf = PathBuf::from(relative_path);
    let target_path = Path::new(&collection.root_path).join(&relative_path_buf);
    let write_id = Uuid::now_v7().to_string();
    let sentinel_name = format!("{write_id}.needs_full_sync");
    let recovery_dir = vault_sync::collection_recovery_dir(
        &vault_sync::recovery_root_for_db_path(Path::new(&db_path)),
        prepared.collection_id,
    );
    let sentinel_path = recovery_dir.join(&sentinel_name);
    let dedup_key = write_dedup_key(&target_path, &prepared.sha256);
    vault_sync::check_update_expected_version(
        prepared.collection_id,
        relative_path,
        prepared.current_version,
        expected_version,
    )?;
    let hooks = test_hooks_snapshot(db);
    let root_fd = match fs_safety::open_root_fd(Path::new(&collection.root_path)) {
        Ok(root_fd) => root_fd,
        Err(error) => {
            return Err(error.into());
        }
    };
    let parent_fd = match fs_safety::walk_to_parent_create_dirs(&root_fd, &relative_path_buf) {
        Ok(parent_fd) => parent_fd,
        Err(error) => {
            return Err(error.into());
        }
    };
    vault_sync::check_fs_precondition_with_parent_fd(
        db,
        prepared.collection_id,
        Path::new(&collection.root_path),
        &relative_path_buf,
        &parent_fd,
    )?;
    // BEGIN IMMEDIATE so the reserved write lock is taken at txn start and the
    // 5s busy_timeout covers cross-process contention, rather than risking a
    // transient SQLITE_BUSY from inside an already-open deferred transaction.
    // Converge with db::with_immediate_transaction once PR #230 lands.
    let tx = match crate::core::db::begin_immediate(db) {
        Ok(tx) => tx,
        Err(error) => return Err(error.into()),
    };
    let staged = match stage_page_record(&tx, prepared, expected_version) {
        Ok(staged) => staged,
        Err(error) => {
            let _ = tx.rollback();
            return Err(error);
        }
    };
    maybe_block_after_supersede_claim(db, prepared.supersedes.as_deref());
    create_recovery_sentinel(prepared, &recovery_dir, &sentinel_name, hooks.as_ref())?;
    let target_name_os = match relative_path_buf.file_name() {
        Some(name) => name,
        None => {
            let _ = tx.rollback();
            let _ = remove_recovery_sentinel(&recovery_dir, &sentinel_name);
            return Err(vault_sync::VaultSyncError::InvariantViolation {
                message: format!("slug={} produced no filename", prepared.slug),
            });
        }
    };
    let target_name = Path::new(target_name_os);
    let temp_name = PathBuf::from(format!(".quaid-write-{write_id}.tmp"));
    let temp_file = match create_tempfile(&parent_fd, &temp_name, raw_bytes) {
        Ok(temp_file) => temp_file,
        Err(error) => {
            let _ = tx.rollback();
            let _ = cleanup_pre_rename(
                &parent_fd,
                &temp_name,
                &target_path,
                &dedup_key,
                &recovery_dir,
                &sentinel_name,
            );
            return Err(error);
        }
    };
    let temp_identity = file_identity(&temp_file)?;
    if let Ok(existing) = fs_safety::stat_at_nofollow(&parent_fd, target_name) {
        if existing.is_symlink() {
            let _ = tx.rollback();
            let _ = cleanup_pre_rename(
                &parent_fd,
                &temp_name,
                &target_path,
                &dedup_key,
                &recovery_dir,
                &sentinel_name,
            );
            return Err(io::Error::other("target path is a symlink").into());
        }
    }

    if let Some(hook) = hooks.as_ref() {
        if let Err(error) = hook.before_rename(&target_path) {
            let _ = tx.rollback();
            let _ = cleanup_pre_rename(
                &parent_fd,
                &temp_name,
                &target_path,
                &dedup_key,
                &recovery_dir,
                &sentinel_name,
            );
            return Err(error);
        }
    }

    if let Err(error) = vault_sync::insert_write_dedup(&dedup_key) {
        let _ = tx.rollback();
        cleanup_pre_rename_without_dedup_clear(
            &parent_fd,
            &temp_name,
            &recovery_dir,
            &sentinel_name,
        );
        return Err(error);
    }
    if let Err(error) = vault_sync::remember_self_write_path(&target_path, &prepared.sha256) {
        let _ = tx.rollback();
        let _ = cleanup_pre_rename(
            &parent_fd,
            &temp_name,
            &target_path,
            &dedup_key,
            &recovery_dir,
            &sentinel_name,
        );
        return Err(error);
    }

    if let Some(hook) = hooks.as_ref() {
        if hook.fail_rename {
            let error = io::Error::other("injected rename failure");
            let _ = tx.rollback();
            let _ = cleanup_pre_rename(
                &parent_fd,
                &temp_name,
                &target_path,
                &dedup_key,
                &recovery_dir,
                &sentinel_name,
            );
            return Err(error.into());
        }
    }

    if let Err(error) = fs_safety::renameat_parent_fd(&parent_fd, &temp_name, target_name) {
        let _ = tx.rollback();
        let _ = cleanup_pre_rename(
            &parent_fd,
            &temp_name,
            &target_path,
            &dedup_key,
            &recovery_dir,
            &sentinel_name,
        );
        return Err(error.into());
    }

    if let Some(hook) = hooks.as_ref() {
        if hook.fail_parent_fsync {
            let _ = tx.rollback();
            return Err(handle_post_rename_failure(
                db,
                prepared,
                relative_path,
                &sentinel_path,
                &target_path,
                &dedup_key,
                "fsync-parent",
                io::Error::other("injected parent fsync failure").to_string(),
            ));
        }
    }
    if let Err(error) = sync_fd(&parent_fd) {
        let _ = tx.rollback();
        return Err(handle_post_rename_failure(
            db,
            prepared,
            relative_path,
            &sentinel_path,
            &target_path,
            &dedup_key,
            "fsync-parent",
            error.to_string(),
        ));
    }

    if let Some(hook) = hooks.as_ref() {
        hook.after_rename(db, &target_path)?;
    }

    let post_rename_stat = match file_state::stat_file_fd(&parent_fd, target_name) {
        Ok(stat) => stat,
        Err(error) => {
            let _ = tx.rollback();
            return Err(handle_post_rename_failure(
                db,
                prepared,
                relative_path,
                &sentinel_path,
                &target_path,
                &dedup_key,
                "post-rename-stat",
                error.to_string(),
            ));
        }
    };

    let final_hash = match file_state::hash_file(&target_path) {
        Ok(hash) => hash,
        Err(error) => {
            let _ = tx.rollback();
            return Err(handle_post_rename_failure(
                db,
                prepared,
                relative_path,
                &sentinel_path,
                &target_path,
                &dedup_key,
                "post-rename-hash",
                error.to_string(),
            ));
        }
    };

    if post_rename_stat.size_bytes != temp_identity.size_bytes
        || post_rename_stat.inode != Some(temp_identity.inode)
        || final_hash != prepared.sha256
    {
        let _ = tx.rollback();
        clear_failure_tracking(&target_path, &dedup_key);
        let _ = maybe_mark_collection_needs_full_sync(db, prepared.collection_id);
        return Err(vault_sync::VaultSyncError::Conflict(
            vault_sync::ConflictError::ConcurrentRename {
                collection_id: prepared.collection_id,
                relative_path: relative_path.to_owned(),
                sentinel_path: sentinel_path.display().to_string(),
            },
        ));
    }

    if matches!(hooks.as_ref(), Some(hook) if hook.fail_commit) {
        let _ = tx.rollback();
        return Err(handle_post_rename_failure(
            db,
            prepared,
            relative_path,
            &sentinel_path,
            &target_path,
            &dedup_key,
            "commit",
            io::Error::other("injected commit failure").to_string(),
        ));
    }

    let outcome = match commit_staged_page_record(
        tx,
        prepared,
        staged,
        raw_bytes,
        relative_path,
        Some(&post_rename_stat),
    ) {
        Ok(outcome) => outcome,
        Err(error) => {
            return Err(handle_post_rename_failure(
                db,
                prepared,
                relative_path,
                &sentinel_path,
                &target_path,
                &dedup_key,
                "commit",
                error.to_string(),
            ));
        }
    };

    let _ = vault_sync::remove_write_dedup(&dedup_key);
    let _ = remove_recovery_sentinel(&recovery_dir, &sentinel_name);
    Ok(outcome)
}

fn slug_to_relative_path(slug: &str) -> String {
    format!("{slug}.md")
}

fn sha256_hex(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    hex::encode(hasher.finalize())
}

#[cfg(unix)]
fn write_dedup_key(target_path: &Path, sha256: &str) -> String {
    format!("{}:{sha256}", target_path.display())
}

#[cfg(unix)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct FileIdentity {
    size_bytes: i64,
    inode: i64,
}

#[cfg(unix)]
fn file_identity(file: &File) -> Result<FileIdentity, vault_sync::VaultSyncError> {
    let metadata = file.metadata()?;
    Ok(FileIdentity {
        size_bytes: metadata.len() as i64,
        inode: metadata.ino() as i64,
    })
}

#[cfg(unix)]
fn create_recovery_sentinel(
    prepared: &PreparedPut,
    recovery_dir: &Path,
    sentinel_name: &str,
    hooks: Option<&PutTestHooks>,
) -> Result<(), vault_sync::VaultSyncError> {
    let sentinel_path = recovery_dir.join(sentinel_name);
    if hooks.is_some_and(|hook| hook.fail_sentinel_create) {
        return Err(vault_sync::VaultSyncError::Watcher(
            vault_sync::WatcherError::RecoverySentinel {
                collection_id: prepared.collection_id,
                relative_path: prepared.slug.clone(),
                sentinel_path: sentinel_path.display().to_string(),
                reason: "injected sentinel creation failure".to_string(),
            },
        ));
    }
    fs::create_dir_all(recovery_dir).map_err(|error| {
        vault_sync::VaultSyncError::Watcher(vault_sync::WatcherError::RecoverySentinel {
            collection_id: prepared.collection_id,
            relative_path: prepared.slug.clone(),
            sentinel_path: sentinel_path.display().to_string(),
            reason: error.to_string(),
        })
    })?;
    let recovery_fd = fs_safety::open_root_fd(recovery_dir).map_err(|error| {
        vault_sync::VaultSyncError::Watcher(vault_sync::WatcherError::RecoverySentinel {
            collection_id: prepared.collection_id,
            relative_path: prepared.slug.clone(),
            sentinel_path: sentinel_path.display().to_string(),
            reason: error.to_string(),
        })
    })?;
    let mut sentinel_file = File::from(
        fs_safety::openat_create_excl(&recovery_fd, Path::new(sentinel_name)).map_err(|error| {
            vault_sync::VaultSyncError::Watcher(vault_sync::WatcherError::RecoverySentinel {
                collection_id: prepared.collection_id,
                relative_path: prepared.slug.clone(),
                sentinel_path: sentinel_path.display().to_string(),
                reason: error.to_string(),
            })
        })?,
    );
    if let Err(error) = sentinel_file
        .write_all(b"dirty\n")
        .and_then(|_| sentinel_file.sync_all())
        .and_then(|_| sync_fd(&recovery_fd))
    {
        let _ = fs_safety::unlinkat_parent_fd(&recovery_fd, Path::new(sentinel_name));
        let _ = sync_fd(&recovery_fd);
        return Err(vault_sync::VaultSyncError::Watcher(
            vault_sync::WatcherError::RecoverySentinel {
                collection_id: prepared.collection_id,
                relative_path: prepared.slug.clone(),
                sentinel_path: sentinel_path.display().to_string(),
                reason: error.to_string(),
            },
        ));
    }
    Ok(())
}

#[cfg(unix)]
fn create_tempfile<Fd: AsFd>(
    parent_fd: Fd,
    temp_name: &Path,
    raw_bytes: &[u8],
) -> Result<File, vault_sync::VaultSyncError> {
    let mut temp_file = File::from(fs_safety::openat_create_excl(parent_fd, temp_name)?);
    temp_file.write_all(raw_bytes)?;
    temp_file.sync_all()?;
    Ok(temp_file)
}

#[cfg(unix)]
fn cleanup_pre_rename(
    parent_fd: &impl AsFd,
    temp_name: &Path,
    target_path: &Path,
    dedup_key: &str,
    recovery_dir: &Path,
    sentinel_name: &str,
) -> Result<(), vault_sync::VaultSyncError> {
    let _ = fs_safety::unlinkat_parent_fd(parent_fd, temp_name);
    clear_failure_tracking(target_path, dedup_key);
    let _ = remove_recovery_sentinel(recovery_dir, sentinel_name);
    Ok(())
}

#[cfg(unix)]
fn cleanup_pre_rename_without_dedup_clear(
    parent_fd: &impl AsFd,
    temp_name: &Path,
    recovery_dir: &Path,
    sentinel_name: &str,
) {
    let _ = fs_safety::unlinkat_parent_fd(parent_fd, temp_name);
    let _ = remove_recovery_sentinel(recovery_dir, sentinel_name);
}

#[cfg(unix)]
fn remove_recovery_sentinel(
    recovery_dir: &Path,
    sentinel_name: &str,
) -> Result<(), vault_sync::VaultSyncError> {
    let recovery_fd = fs_safety::open_root_fd(recovery_dir)?;
    let _ = fs_safety::unlinkat_parent_fd(&recovery_fd, Path::new(sentinel_name));
    let _ = sync_fd(&recovery_fd);
    Ok(())
}

#[cfg(unix)]
#[expect(
    clippy::too_many_arguments,
    reason = "post-rename failure handler binds the full recovery context (db, prepared put, paths, sentinel, dedup keys); collapsing into a struct here would obscure the call site"
)]
fn handle_post_rename_failure(
    db: &Connection,
    prepared: &PreparedPut,
    relative_path: &str,
    sentinel_path: &Path,
    target_path: &Path,
    dedup_key: &str,
    stage: &'static str,
    reason: String,
) -> vault_sync::VaultSyncError {
    clear_failure_tracking(target_path, dedup_key);
    let _ = maybe_mark_collection_needs_full_sync(db, prepared.collection_id);
    vault_sync::VaultSyncError::Watcher(vault_sync::WatcherError::PostRenameRecoveryPending {
        collection_id: prepared.collection_id,
        relative_path: relative_path.to_owned(),
        sentinel_path: sentinel_path.display().to_string(),
        stage,
        reason,
    })
}

#[cfg(unix)]
fn clear_failure_tracking(target_path: &Path, dedup_key: &str) {
    let _ = vault_sync::remove_write_dedup(dedup_key);
    let _ = vault_sync::forget_self_write_path(target_path);
}

#[cfg(unix)]
fn sync_fd<Fd: AsFd>(fd: Fd) -> io::Result<()> {
    fsync(fd).map_err(|error| io::Error::from_raw_os_error(error.raw_os_error()))
}

#[cfg(not(test))]
#[cfg(unix)]
fn test_hooks_snapshot(_db: &Connection) -> Option<PutTestHooks> {
    None
}

#[cfg(all(test, unix))]
fn test_hooks_snapshot(db: &Connection) -> Option<PutTestHooks> {
    let db_path = vault_sync::database_path(db).ok()?;
    put_test_hooks()
        .lock()
        .unwrap_or_else(|err| err.into_inner())
        .get(&db_path)
        .cloned()
}

#[cfg(all(test, unix))]
fn put_test_hooks() -> &'static std::sync::Mutex<std::collections::HashMap<String, PutTestHooks>> {
    static HOOKS: std::sync::OnceLock<
        std::sync::Mutex<std::collections::HashMap<String, PutTestHooks>>,
    > = std::sync::OnceLock::new();
    HOOKS.get_or_init(|| std::sync::Mutex::new(std::collections::HashMap::new()))
}

#[cfg(all(unix, not(all(test, unix))))]
fn maybe_mark_collection_needs_full_sync(
    db: &Connection,
    collection_id: i64,
) -> Result<(), vault_sync::VaultSyncError> {
    vault_sync::mark_collection_needs_full_sync_via_fresh_connection(db, collection_id)
}

#[cfg(all(test, unix))]
fn maybe_mark_collection_needs_full_sync(
    db: &Connection,
    collection_id: i64,
) -> Result<(), vault_sync::VaultSyncError> {
    if matches!(test_hooks_snapshot(db), Some(hooks) if hooks.skip_dirty_mark) {
        return Ok(());
    }
    vault_sync::mark_collection_needs_full_sync_via_fresh_connection(db, collection_id)
}

#[cfg(not(all(test, unix)))]
fn maybe_block_inside_write_lock(_db: &Connection) {}

#[cfg(all(test, unix))]
#[derive(Debug, Default)]
struct WriteLockBlockState {
    blocked_once: bool,
    entered: bool,
    release: bool,
}

#[cfg(all(test, unix))]
#[derive(Debug, Default)]
struct SupersedeClaimBlockState {
    blocked_once: bool,
    entered: bool,
    release: bool,
}

#[cfg(all(test, unix))]
type WriteLockBlocker = std::sync::Arc<(std::sync::Mutex<WriteLockBlockState>, std::sync::Condvar)>;

#[cfg(all(test, unix))]
type WriteLockBlockerMap = std::sync::Mutex<std::collections::HashMap<String, WriteLockBlocker>>;

#[cfg(all(test, unix))]
type SupersedeClaimBlocker = std::sync::Arc<(
    std::sync::Mutex<SupersedeClaimBlockState>,
    std::sync::Condvar,
)>;

#[cfg(all(test, unix))]
type SupersedeClaimBlockerMap =
    std::sync::Mutex<std::collections::HashMap<String, SupersedeClaimBlocker>>;

#[cfg(all(test, unix))]
fn write_lock_blockers() -> &'static WriteLockBlockerMap {
    static BLOCKERS: std::sync::OnceLock<WriteLockBlockerMap> = std::sync::OnceLock::new();
    BLOCKERS.get_or_init(|| std::sync::Mutex::new(std::collections::HashMap::new()))
}

#[cfg(all(test, unix))]
fn write_lock_blocker_for_path(db_path: &str) -> WriteLockBlocker {
    write_lock_blockers()
        .lock()
        .unwrap()
        .entry(db_path.to_owned())
        .or_insert_with(|| {
            std::sync::Arc::new((
                std::sync::Mutex::new(WriteLockBlockState::default()),
                std::sync::Condvar::new(),
            ))
        })
        .clone()
}

#[cfg(all(test, unix))]
fn supersede_claim_blockers() -> &'static SupersedeClaimBlockerMap {
    static BLOCKERS: std::sync::OnceLock<SupersedeClaimBlockerMap> = std::sync::OnceLock::new();
    BLOCKERS.get_or_init(|| std::sync::Mutex::new(std::collections::HashMap::new()))
}

#[cfg(all(test, unix))]
fn supersede_claim_blocker_for_path(db_path: &str) -> SupersedeClaimBlocker {
    supersede_claim_blockers()
        .lock()
        .unwrap()
        .entry(db_path.to_owned())
        .or_insert_with(|| {
            std::sync::Arc::new((
                std::sync::Mutex::new(SupersedeClaimBlockState::default()),
                std::sync::Condvar::new(),
            ))
        })
        .clone()
}

#[cfg(all(test, unix))]
fn maybe_block_inside_write_lock(db: &Connection) {
    let Some(hooks) = test_hooks_snapshot(db) else {
        return;
    };
    if !hooks.block_inside_slug_lock {
        return;
    }
    let db_path = vault_sync::database_path(db).expect("test blocker requires database path");
    let blocker = write_lock_blocker_for_path(&db_path);
    let (state_lock, wakeup) = &*blocker;
    let mut state = state_lock.lock().unwrap();
    if state.blocked_once {
        return;
    }
    state.blocked_once = true;
    state.entered = true;
    wakeup.notify_all();
    while !state.release {
        state = wakeup.wait(state).unwrap();
    }
}

#[cfg(not(all(test, unix)))]
fn maybe_block_after_supersede_claim(_db: &Connection, _supersedes: Option<&str>) {}

#[cfg(all(test, unix))]
fn maybe_block_after_supersede_claim(db: &Connection, supersedes: Option<&str>) {
    if supersedes.is_none() {
        return;
    }
    let Some(hooks) = test_hooks_snapshot(db) else {
        return;
    };
    if !hooks.block_after_supersede_claim {
        return;
    }
    let db_path = vault_sync::database_path(db).expect("test blocker requires database path");
    let blocker = supersede_claim_blocker_for_path(&db_path);
    let (state_lock, wakeup) = &*blocker;
    let mut state = state_lock.lock().unwrap();
    if state.blocked_once {
        return;
    }
    state.blocked_once = true;
    state.entered = true;
    wakeup.notify_all();
    while !state.release {
        state = wakeup.wait(state).unwrap();
    }
}

// reason: white-box; needs PutTestHooks, WriteLockBlockState, SupersedeClaimBlockState,
// put_test_hooks, write_lock_blocker_for_path, supersede_claim_blocker_for_path,
// put_from_cli_string, write_dedup_key, sha256_hex, vault_sync::has_write_dedup
// (cfg(test)-gated) — public-API tests have been moved to tests/cli_put_*.rs.
#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::db;
    #[cfg(all(unix, target_os = "linux"))]
    use std::ffi::OsString;
    use std::fs as stdfs;
    use std::path::Path;
    #[cfg(unix)]
    use std::path::PathBuf;
    #[cfg(unix)]
    use std::thread;
    #[cfg(unix)]
    use std::time::{Duration, Instant};

    #[cfg(all(unix, target_os = "linux"))]
    static ENV_MUTATION_LOCK: std::sync::OnceLock<std::sync::Mutex<()>> =
        std::sync::OnceLock::new();

    #[cfg(all(unix, target_os = "linux"))]
    fn env_mutation_lock() -> &'static std::sync::Mutex<()> {
        ENV_MUTATION_LOCK.get_or_init(|| std::sync::Mutex::new(()))
    }

    #[cfg(all(unix, target_os = "linux"))]
    fn secure_runtime_root() -> tempfile::TempDir {
        use std::os::unix::fs::PermissionsExt;

        let dir = tempfile::TempDir::new().unwrap();
        std::fs::set_permissions(dir.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
        dir
    }

    #[cfg(all(unix, target_os = "linux"))]
    struct EnvVarGuard {
        key: &'static str,
        previous: Option<OsString>,
    }

    #[cfg(all(unix, target_os = "linux"))]
    impl EnvVarGuard {
        fn set(key: &'static str, value: &str) -> Self {
            let previous = std::env::var_os(key);
            #[expect(
                unsafe_code,
                reason = "std::env::set_var is unsafe on Rust 1.81+; tests serialise mutations via the surrounding env-mutation lock"
            )]
            unsafe {
                std::env::set_var(key, value);
            }
            Self { key, previous }
        }
    }

    #[cfg(all(unix, target_os = "linux"))]
    impl Drop for EnvVarGuard {
        fn drop(&mut self) {
            #[expect(
                unsafe_code,
                reason = "std::env::set_var/remove_var are unsafe on Rust 1.81+; restores the previous value inside the same locked window as the constructor"
            )]
            unsafe {
                if let Some(value) = self.previous.as_ref() {
                    std::env::set_var(self.key, value);
                } else {
                    std::env::remove_var(self.key);
                }
            }
        }
    }

    #[cfg(unix)]
    fn open_test_db_with_vault() -> (tempfile::TempDir, String, Connection, PathBuf) {
        let dir = tempfile::TempDir::new().unwrap();
        // Canonicalize once: macOS's /var → /private/var symlink would otherwise cause
        // production-side path lookups (PRAGMA database_list, fs::canonicalize on
        // collection roots) to disagree with the symlinked TempDir path the test stores.
        let canonical_dir = std::fs::canonicalize(dir.path()).unwrap();
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&canonical_dir, std::fs::Permissions::from_mode(0o700))
                .unwrap();
        }
        let db_path = canonical_dir.join("test_memory.db");
        let conn = db::open(db_path.to_str().unwrap()).unwrap();
        conn.busy_timeout(Duration::from_millis(0)).unwrap();
        let vault_root = canonical_dir.join("vault");
        std::fs::create_dir_all(&vault_root).unwrap();
        conn.execute(
            "UPDATE collections
             SET root_path = ?1,
                 writable = 1,
                 is_write_target = 1,
                 state = 'active',
                 needs_full_sync = 0
             WHERE id = 1",
            [vault_root.display().to_string()],
        )
        .unwrap();
        (dir, db_path.display().to_string(), conn, vault_root)
    }

    #[cfg(unix)]
    fn recovery_sentinel_count(db_path: &str, collection_id: i64) -> usize {
        let recovery_root = vault_sync::recovery_root_for_db_path(Path::new(db_path));
        std::fs::read_dir(vault_sync::collection_recovery_dir(
            &recovery_root,
            collection_id,
        ))
        .map(|entries| {
            entries
                .filter_map(Result::ok)
                .filter(|entry| {
                    entry
                        .file_name()
                        .to_string_lossy()
                        .ends_with(".needs_full_sync")
                })
                .count()
        })
        .unwrap_or(0)
    }

    #[cfg(unix)]
    fn collection_needs_full_sync(conn: &Connection, collection_id: i64) -> i64 {
        conn.query_row(
            "SELECT needs_full_sync FROM collections WHERE id = ?1",
            [collection_id],
            |row| row.get(0),
        )
        .unwrap()
    }

    #[cfg(unix)]
    fn page_count(conn: &Connection, slug: &str) -> i64 {
        conn.query_row(
            "SELECT COUNT(*) FROM pages WHERE slug = ?1",
            [slug],
            |row| row.get(0),
        )
        .unwrap()
    }

    #[cfg(unix)]
    fn hook_test_lock() -> &'static std::sync::Mutex<()> {
        static LOCK: std::sync::OnceLock<std::sync::Mutex<()>> = std::sync::OnceLock::new();
        LOCK.get_or_init(|| std::sync::Mutex::new(()))
    }

    #[cfg(unix)]
    struct HookGuard {
        _guard: std::sync::MutexGuard<'static, ()>,
        db_path: String,
    }

    #[cfg(unix)]
    impl HookGuard {
        fn acquire(db_path: impl Into<String>) -> Self {
            let db_path = db_path.into();
            let guard = hook_test_lock()
                .lock()
                .unwrap_or_else(|err| err.into_inner());
            put_test_hooks()
                .lock()
                .unwrap_or_else(|err| err.into_inner())
                .insert(db_path.clone(), PutTestHooks::default());
            Self {
                _guard: guard,
                db_path,
            }
        }

        fn set(&self, hooks: PutTestHooks) {
            put_test_hooks()
                .lock()
                .unwrap_or_else(|err| err.into_inner())
                .insert(self.db_path.clone(), hooks);
        }
    }

    #[cfg(unix)]
    impl Drop for HookGuard {
        fn drop(&mut self) {
            put_test_hooks()
                .lock()
                .unwrap_or_else(|err| err.into_inner())
                .remove(&self.db_path);
        }
    }

    #[cfg(unix)]
    fn reset_write_lock_blocker(db_path: &str) {
        let blocker = write_lock_blocker_for_path(db_path);
        let (state_lock, _) = &*blocker;
        *state_lock.lock().unwrap() = WriteLockBlockState::default();
    }

    #[cfg(unix)]
    fn reset_supersede_claim_blocker(db_path: &str) {
        let blocker = supersede_claim_blocker_for_path(db_path);
        let (state_lock, _) = &*blocker;
        *state_lock.lock().unwrap() = SupersedeClaimBlockState::default();
    }

    #[cfg(unix)]
    fn wait_for_write_lock_entry(db_path: &str) {
        let blocker = write_lock_blocker_for_path(db_path);
        let (state_lock, wakeup) = &*blocker;
        let deadline = Instant::now() + Duration::from_secs(5);
        let mut state = state_lock.lock().unwrap();
        while !state.entered {
            let remaining = deadline.saturating_duration_since(Instant::now());
            assert!(
                !remaining.is_zero(),
                "timed out waiting for first writer to enter slug lock"
            );
            let (next_state, timeout) = wakeup.wait_timeout(state, remaining).unwrap();
            state = next_state;
            assert!(
                !timeout.timed_out() || state.entered,
                "timed out waiting for first writer to enter slug lock"
            );
        }
    }

    #[cfg(unix)]
    fn release_write_lock_blocker(db_path: &str) {
        let blocker = write_lock_blocker_for_path(db_path);
        let (state_lock, wakeup) = &*blocker;
        let mut state = state_lock.lock().unwrap();
        state.release = true;
        wakeup.notify_all();
    }

    #[cfg(unix)]
    fn wait_for_supersede_claim_entry(db_path: &str) {
        let blocker = supersede_claim_blocker_for_path(db_path);
        let (state_lock, wakeup) = &*blocker;
        let deadline = Instant::now() + Duration::from_secs(5);
        let mut state = state_lock.lock().unwrap();
        while !state.entered {
            let remaining = deadline.saturating_duration_since(Instant::now());
            assert!(
                !remaining.is_zero(),
                "timed out waiting for supersede claim entry"
            );
            let (next_state, timeout) = wakeup.wait_timeout(state, remaining).unwrap();
            state = next_state;
            assert!(
                !timeout.timed_out() || state.entered,
                "timed out waiting for supersede claim entry"
            );
        }
    }

    #[cfg(unix)]
    fn release_supersede_claim_blocker(db_path: &str) {
        let blocker = supersede_claim_blocker_for_path(db_path);
        let (state_lock, wakeup) = &*blocker;
        let mut state = state_lock.lock().unwrap();
        state.release = true;
        wakeup.notify_all();
    }

    #[cfg(unix)]
    fn open_test_db_with_vault_guarded(
    ) -> (HookGuard, tempfile::TempDir, String, Connection, PathBuf) {
        let (dir, db_path, conn, vault_root) = open_test_db_with_vault();
        let guard = HookGuard::acquire(db_path.clone());
        (guard, dir, db_path, conn, vault_root)
    }

    #[cfg(all(unix, target_os = "linux"))]
    fn spawn_fake_ipc_server(socket_path: &Path, session_id: &str) -> std::thread::JoinHandle<()> {
        use std::os::unix::fs::PermissionsExt;
        use std::os::unix::net::UnixListener;

        if let Some(parent) = socket_path.parent() {
            std::fs::create_dir_all(parent).unwrap();
            std::fs::set_permissions(parent, std::fs::Permissions::from_mode(0o700)).unwrap();
        }
        if socket_path.exists() {
            std::fs::remove_file(socket_path).unwrap();
        }
        let listener = UnixListener::bind(socket_path).unwrap();
        std::fs::set_permissions(socket_path, std::fs::Permissions::from_mode(0o600)).unwrap();
        let session_id = session_id.to_owned();
        std::thread::spawn(move || {
            let (stream, _) = listener.accept().unwrap();
            let read_stream = stream.try_clone().unwrap();
            let mut reader = BufReader::new(read_stream);
            let mut writer = stream;
            loop {
                let mut line = String::new();
                let bytes = reader.read_line(&mut line).unwrap();
                if bytes == 0 {
                    break;
                }
                let request: vault_sync::IpcRequest =
                    serde_json::from_str(line.trim_end()).unwrap();
                match request {
                    vault_sync::IpcRequest::WhoAmI => {
                        serde_json::to_writer(
                            &mut writer,
                            &vault_sync::IpcResponse::WhoAmI {
                                session_id: session_id.clone(),
                            },
                        )
                        .unwrap();
                        writer.write_all(b"\n").unwrap();
                        writer.flush().unwrap();
                    }
                    vault_sync::IpcRequest::Put { .. } => {
                        serde_json::to_writer(
                            &mut writer,
                            &vault_sync::IpcResponse::PutOk {
                                status: "spoofed".to_owned(),
                            },
                        )
                        .unwrap();
                        writer.write_all(b"\n").unwrap();
                        writer.flush().unwrap();
                        break;
                    }
                }
            }
        })
    }

    fn active_raw_import_count_for_slug(conn: &Connection, slug: &str) -> i64 {
        conn.query_row(
            "SELECT COUNT(*) FROM raw_imports \
             WHERE page_id = (SELECT id FROM pages WHERE slug = ?1) AND is_active = 1",
            [slug],
            |row| row.get(0),
        )
        .unwrap()
    }

    fn active_raw_import_bytes_for_slug(conn: &Connection, slug: &str) -> Vec<u8> {
        conn.query_row(
            "SELECT raw_bytes FROM raw_imports \
             WHERE page_id = (SELECT id FROM pages WHERE slug = ?1) AND is_active = 1",
            [slug],
            |row| row.get(0),
        )
        .unwrap()
    }

    /// Helper: read a page back from the database.
    fn read_page(conn: &Connection, slug: &str) -> Option<(i64, String, String, String, String)> {
        conn.prepare(
            "SELECT version, type, title, compiled_truth, timeline FROM pages WHERE slug = ?1",
        )
        .unwrap()
        .query_row([slug], |row| {
            let compiled_truth: String = row.get(3)?;
            Ok((
                row.get(0)?,
                row.get(1)?,
                row.get(2)?,
                compiled_truth.trim_end_matches('\n').to_owned(),
                row.get(4)?,
            ))
        })
        .ok()
    }

    fn superseded_by_for_slug(conn: &Connection, slug: &str) -> Option<i64> {
        conn.query_row(
            "SELECT superseded_by FROM pages WHERE slug = ?1",
            [slug],
            |row| row.get(0),
        )
        .ok()
        .flatten()
    }

    fn page_id_for_slug(conn: &Connection, slug: &str) -> i64 {
        conn.query_row("SELECT id FROM pages WHERE slug = ?1", [slug], |row| {
            row.get(0)
        })
        .unwrap()
    }

    #[cfg(unix)]
    #[test]
    fn unix_non_head_supersede_rejects_before_write_through_mutation() {
        let (_guard, _dir, db_path, conn, vault_root) = open_test_db_with_vault_guarded();
        put_from_string(
            &conn,
            "facts/a",
            "---\ntitle: A\ntype: fact\n---\nA\n",
            None,
        )
        .unwrap();
        put_from_string(
            &conn,
            "facts/b",
            "---\ntitle: B\ntype: fact\nsupersedes: facts/a\n---\nB\n",
            None,
        )
        .unwrap();

        let a_path = vault_root.join("facts").join("a.md");
        let b_path = vault_root.join("facts").join("b.md");
        let a_disk_before = std::fs::read_to_string(&a_path).unwrap();
        let b_disk_before = std::fs::read_to_string(&b_path).unwrap();
        let a_raw_before = active_raw_import_bytes_for_slug(&conn, "facts/a");
        let b_raw_before = active_raw_import_bytes_for_slug(&conn, "facts/b");

        let error = put_from_string(
            &conn,
            "facts/c",
            "---\ntitle: C\ntype: fact\nsupersedes: facts/a\n---\nC\n",
            None,
        )
        .unwrap_err();

        assert!(error.to_string().contains("SupersedeConflictError"));
        assert!(!error.to_string().contains("PostRenameRecoveryPendingError"));
        assert_eq!(page_count(&conn, "facts/c"), 0);
        assert!(!vault_root.join("facts").join("c.md").exists());
        assert_eq!(std::fs::read_to_string(a_path).unwrap(), a_disk_before);
        assert_eq!(std::fs::read_to_string(b_path).unwrap(), b_disk_before);
        assert_eq!(
            active_raw_import_bytes_for_slug(&conn, "facts/a"),
            a_raw_before
        );
        assert_eq!(
            active_raw_import_bytes_for_slug(&conn, "facts/b"),
            b_raw_before
        );
        assert_eq!(active_raw_import_count_for_slug(&conn, "facts/a"), 1);
        assert_eq!(active_raw_import_count_for_slug(&conn, "facts/b"), 1);
        assert_eq!(collection_needs_full_sync(&conn, 1), 0);
        assert_eq!(recovery_sentinel_count(&db_path, 1), 0);
        assert_eq!(
            superseded_by_for_slug(&conn, "facts/a"),
            Some(page_id_for_slug(&conn, "facts/b"))
        );
    }

    #[cfg(unix)]
    #[test]
    fn concurrent_supersede_contenders_claim_head_before_write_through_and_loser_never_hits_disk() {
        let (guard, _dir, db_path, conn, vault_root) = open_test_db_with_vault_guarded();
        put_from_string(
            &conn,
            "facts/a",
            "---\ntitle: A\ntype: fact\n---\nA\n",
            None,
        )
        .unwrap();

        let a_path = vault_root.join("facts").join("a.md");
        let a_disk_before = std::fs::read_to_string(&a_path).unwrap();
        let a_raw_before = active_raw_import_bytes_for_slug(&conn, "facts/a");

        reset_supersede_claim_blocker(&db_path);
        guard.set(PutTestHooks {
            block_after_supersede_claim: true,
            ..PutTestHooks::default()
        });

        let winner_db_path = db_path.clone();
        let winner = thread::spawn(move || {
            let conn = Connection::open(&winner_db_path).unwrap();
            conn.busy_timeout(Duration::from_secs(2)).unwrap();
            put_from_string(
                &conn,
                "facts/b",
                "---\ntitle: B\ntype: fact\nsupersedes: facts/a\n---\nB\n",
                None,
            )
        });
        wait_for_supersede_claim_entry(&db_path);

        assert_eq!(page_count(&conn, "facts/b"), 0);
        assert!(!vault_root.join("facts").join("b.md").exists());
        assert!(!vault_root.join("facts").join("c.md").exists());
        assert_eq!(std::fs::read_to_string(&a_path).unwrap(), a_disk_before);
        assert_eq!(
            active_raw_import_bytes_for_slug(&conn, "facts/a"),
            a_raw_before
        );

        let (done_tx, done_rx) = std::sync::mpsc::channel();
        let loser_db_path = db_path.clone();
        let loser = thread::spawn(move || {
            let conn = Connection::open(&loser_db_path).unwrap();
            conn.busy_timeout(Duration::from_secs(2)).unwrap();
            let result = put_from_string(
                &conn,
                "facts/c",
                "---\ntitle: C\ntype: fact\nsupersedes: facts/a\n---\nC\n",
                None,
            );
            done_tx.send(result.is_ok()).unwrap();
            result
        });

        match done_rx.recv_timeout(Duration::from_millis(200)) {
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) | Ok(false) => {}
            Ok(true) => {
                panic!("second contender unexpectedly succeeded before the winner finished")
            }
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => {
                panic!("second contender channel disconnected before reporting its result")
            }
        }
        assert!(!vault_root.join("facts").join("c.md").exists());
        assert_eq!(page_count(&conn, "facts/c"), 0);

        release_supersede_claim_blocker(&db_path);

        winner.join().unwrap().unwrap();
        let _error = loser.join().unwrap().unwrap_err();

        let b_path = vault_root.join("facts").join("b.md");
        assert_eq!(std::fs::read_to_string(&a_path).unwrap(), a_disk_before);
        assert_eq!(
            std::fs::read_to_string(&b_path).unwrap(),
            "---\ntitle: B\ntype: fact\nsupersedes: facts/a\n---\nB\n"
        );
        assert!(!vault_root.join("facts").join("c.md").exists());
        assert_eq!(
            active_raw_import_bytes_for_slug(&conn, "facts/a"),
            a_raw_before
        );
        assert_eq!(active_raw_import_count_for_slug(&conn, "facts/a"), 1);
        assert_eq!(active_raw_import_count_for_slug(&conn, "facts/b"), 1);
        assert_eq!(active_raw_import_count_for_slug(&conn, "facts/c"), 0);
        assert_eq!(page_count(&conn, "facts/c"), 0);
        assert_eq!(collection_needs_full_sync(&conn, 1), 0);
        assert_eq!(recovery_sentinel_count(&db_path, 1), 0);
        assert_eq!(
            superseded_by_for_slug(&conn, "facts/a"),
            Some(page_id_for_slug(&conn, "facts/b"))
        );
    }

    #[cfg(unix)]
    #[test]
    fn unix_update_without_expected_version_conflicts_before_sentinel_creation() {
        let (_guard, _dir, db_path, conn, vault_root) = open_test_db_with_vault_guarded();
        put_from_string(
            &conn,
            "notes/missing-expected",
            "---\ntitle: Missing Expected\ntype: note\n---\nOld body\n",
            None,
        )
        .unwrap();

        let error = put_from_string(
            &conn,
            "notes/missing-expected",
            "---\ntitle: Missing Expected\ntype: note\n---\nNew body\n",
            None,
        )
        .unwrap_err();

        assert!(error.to_string().contains("ConflictError"));
        assert!(error.to_string().contains("MissingExpectedVersion"));
        assert_eq!(recovery_sentinel_count(&db_path, 1), 0);
        assert_eq!(
            std::fs::read_to_string(vault_root.join("notes").join("missing-expected.md")).unwrap(),
            "---\ntitle: Missing Expected\ntype: note\n---\nOld body\n"
        );
        assert_eq!(
            read_page(&conn, "notes/missing-expected").unwrap().3,
            "Old body"
        );
    }

    #[cfg(unix)]
    #[test]
    fn unix_stale_expected_version_conflicts_before_sentinel_creation() {
        let (_guard, _dir, db_path, conn, vault_root) = open_test_db_with_vault_guarded();
        put_from_string(
            &conn,
            "notes/stale-expected",
            "---\ntitle: Stale Expected\ntype: note\n---\nOld body\n",
            None,
        )
        .unwrap();

        let error = put_from_string(
            &conn,
            "notes/stale-expected",
            "---\ntitle: Stale Expected\ntype: note\n---\nNew body\n",
            Some(0),
        )
        .unwrap_err();

        assert!(error.to_string().contains("ConflictError"));
        assert!(error.to_string().contains("StaleExpectedVersion"));
        assert_eq!(recovery_sentinel_count(&db_path, 1), 0);
        assert_eq!(
            std::fs::read_to_string(vault_root.join("notes").join("stale-expected.md")).unwrap(),
            "---\ntitle: Stale Expected\ntype: note\n---\nOld body\n"
        );
        assert_eq!(read_page(&conn, "notes/stale-expected").unwrap().0, 1);
    }

    #[cfg(unix)]
    #[test]
    fn unix_external_delete_conflicts_before_sentinel_creation() {
        let (_guard, _dir, db_path, conn, vault_root) = open_test_db_with_vault_guarded();
        put_from_string(
            &conn,
            "notes/external-delete",
            "---\ntitle: External Delete\ntype: note\n---\nOld body\n",
            None,
        )
        .unwrap();
        std::fs::remove_file(vault_root.join("notes").join("external-delete.md")).unwrap();

        let error = put_from_string(
            &conn,
            "notes/external-delete",
            "---\ntitle: External Delete\ntype: note\n---\nNew body\n",
            Some(1),
        )
        .unwrap_err();

        assert!(error.to_string().contains("ConflictError"));
        assert!(error.to_string().contains("ExternalDelete"));
        assert_eq!(recovery_sentinel_count(&db_path, 1), 0);
        assert_eq!(page_count(&conn, "notes/external-delete"), 1);
        assert!(!vault_root.join("notes").join("external-delete.md").exists());
    }

    #[cfg(unix)]
    #[test]
    fn unix_external_create_conflicts_before_sentinel_creation() {
        let (_guard, _dir, db_path, conn, vault_root) = open_test_db_with_vault_guarded();
        let target = vault_root.join("notes").join("external-create.md");
        std::fs::create_dir_all(target.parent().unwrap()).unwrap();
        std::fs::write(&target, b"external bytes").unwrap();

        let error = put_from_string(
            &conn,
            "notes/external-create",
            "---\ntitle: External Create\ntype: note\n---\nBody\n",
            None,
        )
        .unwrap_err();

        assert!(error.to_string().contains("ConflictError"));
        assert!(error.to_string().contains("ExternalCreate"));
        assert_eq!(recovery_sentinel_count(&db_path, 1), 0);
        assert_eq!(page_count(&conn, "notes/external-create"), 0);
        assert_eq!(std::fs::read(&target).unwrap(), b"external bytes");
    }

    #[cfg(unix)]
    #[test]
    fn unix_fresh_create_succeeds_without_existing_file_state() {
        let (_guard, _dir, db_path, conn, vault_root) = open_test_db_with_vault_guarded();
        let body = "---\ntitle: Fresh Create\ntype: note\n---\nBody\n";

        put_from_string(&conn, "notes/fresh-create", body, None).unwrap();

        assert_eq!(recovery_sentinel_count(&db_path, 1), 0);
        assert_eq!(
            std::fs::read_to_string(vault_root.join("notes").join("fresh-create.md")).unwrap(),
            body
        );
        assert_eq!(read_page(&conn, "notes/fresh-create").unwrap().0, 1);
    }

    #[cfg(unix)]
    #[test]
    fn same_slug_writes_wait_for_per_slug_mutex() {
        let (guard, _dir, db_path, conn, _vault_root) = open_test_db_with_vault_guarded();
        put_from_string(
            &conn,
            "notes/serialized",
            "---\ntitle: Serialized\ntype: note\n---\nOld body\n",
            None,
        )
        .unwrap();
        reset_write_lock_blocker(&db_path);
        guard.set(PutTestHooks {
            block_inside_slug_lock: true,
            ..PutTestHooks::default()
        });

        let first_db_path = db_path.clone();
        let first = std::thread::spawn(move || {
            let conn = Connection::open(&first_db_path).unwrap();
            conn.busy_timeout(Duration::from_secs(1)).unwrap();
            put_from_string(
                &conn,
                "notes/serialized",
                "---\ntitle: Serialized\ntype: note\n---\nFirst writer body\n",
                Some(1),
            )
        });
        wait_for_write_lock_entry(&db_path);

        let (done_tx, done_rx) = std::sync::mpsc::channel();
        let second_db_path = db_path.clone();
        let second = std::thread::spawn(move || {
            let conn = Connection::open(&second_db_path).unwrap();
            conn.busy_timeout(Duration::from_secs(1)).unwrap();
            let result = put_from_string(
                &conn,
                "notes/serialized",
                "---\ntitle: Serialized\ntype: note\n---\nSecond writer body\n",
                Some(1),
            );
            done_tx.send(result.is_ok()).unwrap();
            result
        });

        assert!(
            done_rx.recv_timeout(Duration::from_millis(200)).is_err(),
            "same-slug second writer should stay blocked until the first writer releases the slug mutex"
        );

        release_write_lock_blocker(&db_path);

        first.join().unwrap().unwrap();
        let error = second.join().unwrap().unwrap_err();
        assert!(error.to_string().contains("StaleExpectedVersion"));
        let (version, _, _, truth, _) = read_page(&conn, "notes/serialized").unwrap();
        assert_eq!(version, 2);
        assert_eq!(truth, "First writer body");
    }

    #[cfg(unix)]
    #[test]
    fn different_slug_writes_do_not_share_per_slug_mutex() {
        let (guard, _dir, db_path, conn, _vault_root) = open_test_db_with_vault_guarded();
        reset_write_lock_blocker(&db_path);
        guard.set(PutTestHooks {
            block_inside_slug_lock: true,
            ..PutTestHooks::default()
        });

        let blocked_db_path = db_path.clone();
        let blocked = std::thread::spawn(move || {
            let conn = Connection::open(&blocked_db_path).unwrap();
            conn.busy_timeout(Duration::from_secs(1)).unwrap();
            put_from_string(
                &conn,
                "notes/alpha",
                "---\ntitle: Alpha\ntype: note\n---\nAlpha body\n",
                None,
            )
        });
        wait_for_write_lock_entry(&db_path);

        let (done_tx, done_rx) = std::sync::mpsc::channel();
        let free_db_path = db_path.clone();
        let free = std::thread::spawn(move || {
            let conn = Connection::open(&free_db_path).unwrap();
            conn.busy_timeout(Duration::from_secs(1)).unwrap();
            let result = put_from_string(
                &conn,
                "notes/beta",
                "---\ntitle: Beta\ntype: note\n---\nBeta body\n",
                None,
            );
            done_tx.send(result.is_ok()).unwrap();
            result
        });

        assert!(
            done_rx.recv_timeout(Duration::from_secs(1)).unwrap(),
            "different-slug writer should not wait on the blocked slug mutex"
        );

        release_write_lock_blocker(&db_path);

        blocked.join().unwrap().unwrap();
        free.join().unwrap().unwrap();
        assert_eq!(read_page(&conn, "notes/alpha").unwrap().3, "Alpha body");
        assert_eq!(read_page(&conn, "notes/beta").unwrap().3, "Beta body");
    }

    #[cfg(unix)]
    #[test]
    fn sentinel_creation_failure_returns_recovery_sentinel_error_without_db_or_vault_mutation() {
        let (guard, _dir, db_path, conn, vault_root) = open_test_db_with_vault_guarded();
        guard.set(PutTestHooks {
            fail_sentinel_create: true,
            ..PutTestHooks::default()
        });

        let error = put_from_string(
            &conn,
            "notes/sentinel-failure",
            "---\ntitle: Sentinel Failure\ntype: note\n---\nBody\n",
            None,
        )
        .unwrap_err();

        assert!(error.to_string().contains("RecoverySentinelError"));
        assert_eq!(page_count(&conn, "notes/sentinel-failure"), 0);
        assert!(!vault_root
            .join("notes")
            .join("sentinel-failure.md")
            .exists());
        assert_eq!(recovery_sentinel_count(&db_path, 1), 0);
        assert!(!vault_sync::has_write_dedup(&write_dedup_key(
            &vault_root.join("notes").join("sentinel-failure.md"),
            &sha256_hex(b"---\ntitle: Sentinel Failure\ntype: note\n---\nBody\n"),
        ))
        .unwrap());
    }

    #[cfg(unix)]
    #[test]
    fn pre_rename_failure_cleans_tempfile_sentinel_and_dedup() {
        let (guard, _dir, db_path, conn, vault_root) = open_test_db_with_vault_guarded();
        guard.set(PutTestHooks {
            fail_before_rename: true,
            ..PutTestHooks::default()
        });
        let body = "---\ntitle: Pre Rename\ntype: note\n---\nBody\n";
        let dedup_key = write_dedup_key(
            &vault_root.join("notes").join("pre-rename.md"),
            &sha256_hex(body.as_bytes()),
        );

        let error = put_from_string(&conn, "notes/pre-rename", body, None).unwrap_err();

        assert!(error.to_string().contains("injected pre-rename failure"));
        assert_eq!(page_count(&conn, "notes/pre-rename"), 0);
        assert!(!vault_root.join("notes").join("pre-rename.md").exists());
        assert_eq!(recovery_sentinel_count(&db_path, 1), 0);
        assert!(!vault_sync::has_write_dedup(&dedup_key).unwrap());
    }

    #[cfg(unix)]
    #[test]
    fn rename_failure_cleans_tempfile_sentinel_and_dedup() {
        let (guard, _dir, db_path, conn, vault_root) = open_test_db_with_vault_guarded();
        guard.set(PutTestHooks {
            fail_rename: true,
            ..PutTestHooks::default()
        });
        let body = "---\ntitle: Rename Failure\ntype: note\n---\nBody\n";
        let dedup_key = write_dedup_key(
            &vault_root.join("notes").join("rename-failure.md"),
            &sha256_hex(body.as_bytes()),
        );

        let error = put_from_string(&conn, "notes/rename-failure", body, None).unwrap_err();

        assert!(error.to_string().contains("injected rename failure"));
        assert_eq!(page_count(&conn, "notes/rename-failure"), 0);
        assert!(!vault_root.join("notes").join("rename-failure.md").exists());
        assert_eq!(recovery_sentinel_count(&db_path, 1), 0);
        assert!(!vault_sync::has_write_dedup(&dedup_key).unwrap());
    }

    #[cfg(unix)]
    #[test]
    fn duplicate_dedup_entry_refuses_before_rename_without_mutating_disk_or_db() {
        let (_guard, _dir, db_path, conn, vault_root) = open_test_db_with_vault_guarded();
        let body = "---\ntitle: Duplicate Dedup\ntype: note\n---\nBody\n";
        let dedup_key = write_dedup_key(
            &vault_root.join("notes").join("duplicate-dedup.md"),
            &sha256_hex(body.as_bytes()),
        );
        vault_sync::insert_write_dedup(&dedup_key).unwrap();

        let error = put_from_string(&conn, "notes/duplicate-dedup", body, None).unwrap_err();

        assert!(error.to_string().contains("DuplicateWriteDedupError"));
        assert_eq!(page_count(&conn, "notes/duplicate-dedup"), 0);
        assert!(!vault_root.join("notes").join("duplicate-dedup.md").exists());
        assert_eq!(recovery_sentinel_count(&db_path, 1), 0);
        assert!(vault_sync::has_write_dedup(&dedup_key).unwrap());
        vault_sync::remove_write_dedup(&dedup_key).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn write_dedup_keys_are_scoped_by_target_path() {
        let key_a = write_dedup_key(Path::new("/tmp/quaid-a/notes/shared.md"), "same-hash");
        let key_b = write_dedup_key(Path::new("/tmp/quaid-b/notes/shared.md"), "same-hash");

        assert_ne!(key_a, key_b);
        vault_sync::insert_write_dedup(&key_a).unwrap();
        vault_sync::insert_write_dedup(&key_b).unwrap();
        assert!(vault_sync::has_write_dedup(&key_a).unwrap());
        assert!(vault_sync::has_write_dedup(&key_b).unwrap());
        vault_sync::remove_write_dedup(&key_a).unwrap();
        vault_sync::remove_write_dedup(&key_b).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn put_test_hooks_are_scoped_to_the_guarded_database() {
        let (guard, _hook_dir, _hook_db_path, hook_conn, hook_vault_root) =
            open_test_db_with_vault_guarded();
        guard.set(PutTestHooks {
            fail_before_rename: true,
            ..PutTestHooks::default()
        });

        let (_plain_dir, _plain_db_path, plain_conn, plain_vault_root) = open_test_db_with_vault();
        let plain_body = "---\ntitle: Scoped Success\ntype: note\n---\nPlain body\n";
        put_from_string(&plain_conn, "notes/scoped-success", plain_body, None).unwrap();

        assert_eq!(
            read_page(&plain_conn, "notes/scoped-success").unwrap().3,
            "Plain body"
        );
        assert_eq!(
            std::fs::read_to_string(plain_vault_root.join("notes").join("scoped-success.md"))
                .unwrap(),
            plain_body
        );

        let hook_body = "---\ntitle: Scoped Failure\ntype: note\n---\nHook body\n";
        let error =
            put_from_string(&hook_conn, "notes/scoped-failure", hook_body, None).unwrap_err();

        assert!(error.to_string().contains("injected pre-rename failure"));
        assert_eq!(page_count(&hook_conn, "notes/scoped-failure"), 0);
        assert!(!hook_vault_root
            .join("notes")
            .join("scoped-failure.md")
            .exists());
    }

    #[cfg(unix)]
    #[test]
    fn parent_fsync_failure_refuses_db_commit_and_retains_sentinel() {
        let (guard, _dir, db_path, conn, vault_root) = open_test_db_with_vault_guarded();
        put_from_string(
            &conn,
            "notes/fsync-parent",
            "---\ntitle: Parent Fsync\ntype: note\n---\nOld body\n",
            None,
        )
        .unwrap();
        guard.set(PutTestHooks {
            fail_parent_fsync: true,
            ..PutTestHooks::default()
        });

        let error = put_from_string(
            &conn,
            "notes/fsync-parent",
            "---\ntitle: Parent Fsync\ntype: note\n---\nNew body\n",
            Some(1),
        )
        .unwrap_err();

        assert!(error.to_string().contains("PostRenameRecoveryPendingError"));
        assert!(error.to_string().contains("stage=fsync-parent"));
        assert_eq!(collection_needs_full_sync(&conn, 1), 1);
        let dedup_key = write_dedup_key(
            &vault_root.join("notes").join("fsync-parent.md"),
            &sha256_hex(b"---\ntitle: Parent Fsync\ntype: note\n---\nNew body\n"),
        );
        assert_eq!(
            read_page(&conn, "notes/fsync-parent").unwrap().3,
            "Old body"
        );
        assert_eq!(recovery_sentinel_count(&db_path, 1), 1);
        assert!(vault_root.join("notes").join("fsync-parent.md").exists());
        assert!(!vault_sync::has_write_dedup(&dedup_key).unwrap());
    }

    #[cfg(unix)]
    #[test]
    fn foreign_rename_returns_concurrent_rename_and_retains_sentinel() {
        let (guard, _dir, db_path, conn, vault_root) = open_test_db_with_vault_guarded();
        put_from_string(
            &conn,
            "notes/concurrent",
            "---\ntitle: Concurrent\ntype: note\n---\nOriginal body\n",
            None,
        )
        .unwrap();
        guard.set(PutTestHooks {
            post_rename_swap: Some(
                b"---\ntitle: Concurrent\ntype: note\n---\nForeign body\n".to_vec(),
            ),
            ..PutTestHooks::default()
        });

        let error = put_from_string(
            &conn,
            "notes/concurrent",
            "---\ntitle: Concurrent\ntype: note\n---\nLocal body\n",
            Some(1),
        )
        .unwrap_err();

        assert!(error.to_string().contains("ConcurrentRenameError"));
        assert_eq!(collection_needs_full_sync(&conn, 1), 1);
        assert_eq!(recovery_sentinel_count(&db_path, 1), 1);
        assert!(vault_root.join("notes").join("concurrent.md").exists());
        assert!(!vault_sync::has_write_dedup(&write_dedup_key(
            &vault_root.join("notes").join("concurrent.md"),
            &sha256_hex(b"---\ntitle: Concurrent\ntype: note\n---\nLocal body\n"),
        ))
        .unwrap());
    }

    #[cfg(unix)]
    #[test]
    fn commit_failure_retains_sentinel_until_startup_recovery_reconciles() {
        let (guard, _dir, db_path, conn, vault_root) = open_test_db_with_vault_guarded();
        put_from_string(
            &conn,
            "notes/busy",
            "---\ntitle: Busy\ntype: note\n---\nOld body\n",
            None,
        )
        .unwrap();
        guard.set(PutTestHooks {
            fail_commit: true,
            ..PutTestHooks::default()
        });

        let error = put_from_string(
            &conn,
            "notes/busy",
            "---\ntitle: Busy\ntype: note\n---\nNew body on disk\n",
            Some(1),
        )
        .unwrap_err();

        assert!(error.to_string().contains("PostRenameRecoveryPendingError"));
        assert!(error.to_string().contains("stage=commit"));
        assert_eq!(recovery_sentinel_count(&db_path, 1), 1);
        assert!(!vault_sync::has_write_dedup(&write_dedup_key(
            &vault_root.join("notes").join("busy.md"),
            &sha256_hex(b"---\ntitle: Busy\ntype: note\n---\nNew body on disk\n"),
        ))
        .unwrap());

        let runtime = vault_sync::start_serve_runtime(db_path.clone()).unwrap();
        let recovered = wait_for_recovered_truth(&db_path, "notes/busy", "New body on disk");
        assert!(recovered);
        drop(runtime);

        let verify = Connection::open(&db_path).unwrap();
        assert_eq!(collection_needs_full_sync(&verify, 1), 0);
        assert_eq!(recovery_sentinel_count(&db_path, 1), 0);
    }

    #[cfg(unix)]
    #[test]
    fn foreign_rename_without_dirty_mark_falls_back_to_startup_sentinel_recovery() {
        let (guard, _dir, db_path, conn, _vault_root) = open_test_db_with_vault_guarded();
        put_from_string(
            &conn,
            "notes/foreign-busy",
            "---\ntitle: Foreign Busy\ntype: note\n---\nOld body\n",
            None,
        )
        .unwrap();
        guard.set(PutTestHooks {
            skip_dirty_mark: true,
            post_rename_swap: Some(
                b"---\ntitle: Foreign Busy\ntype: note\n---\nForeign winner body\n".to_vec(),
            ),
            ..PutTestHooks::default()
        });

        let error = put_from_string(
            &conn,
            "notes/foreign-busy",
            "---\ntitle: Foreign Busy\ntype: note\n---\nLocal loser body\n",
            Some(1),
        )
        .unwrap_err();

        assert!(error.to_string().contains("ConcurrentRenameError"));
        assert_eq!(recovery_sentinel_count(&db_path, 1), 1);
        drop(guard);

        let verify_before_runtime = Connection::open(&db_path).unwrap();
        assert_eq!(collection_needs_full_sync(&verify_before_runtime, 1), 0);
        drop(verify_before_runtime);

        let runtime = vault_sync::start_serve_runtime(db_path.clone()).unwrap();
        let recovered =
            wait_for_recovered_truth(&db_path, "notes/foreign-busy", "Foreign winner body");
        assert!(recovered);
        drop(runtime);

        let verify = Connection::open(&db_path).unwrap();
        assert_eq!(collection_needs_full_sync(&verify, 1), 0);
        assert_eq!(recovery_sentinel_count(&db_path, 1), 0);
    }

    #[cfg(unix)]
    fn wait_for_recovered_truth(db_path: &str, slug: &str, expected: &str) -> bool {
        let started = std::time::Instant::now();
        while started.elapsed() < Duration::from_secs(5) {
            let verify = Connection::open(db_path).unwrap();
            let outcome: Option<(String, i64)> = verify
                .query_row(
                    "SELECT compiled_truth, version FROM pages WHERE slug = ?1",
                    [slug],
                    |row| Ok((row.get(0)?, row.get(1)?)),
                )
                .ok();
            if let Some((truth, version)) = outcome {
                if truth.contains(expected) && version >= 2 {
                    return true;
                }
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        false
    }

    #[cfg(unix)]
    #[test]
    fn cli_put_does_not_refuse_cli_session_as_serve_owner() {
        let (_guard, _dir, _db_path, conn, vault_root) = open_test_db_with_vault_guarded();
        // A live CLI-type session in collection_owners (e.g. a concurrent offline put) must NOT
        // cause put_from_cli_string to return RuntimeOwnsCollectionError.
        conn.execute(
            "INSERT INTO serve_sessions (session_id, pid, host, heartbeat_at, session_type)
             VALUES ('cli-offline', 9999, 'cli-host', datetime('now'), 'cli')",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO collection_owners (collection_id, session_id) VALUES (1, 'cli-offline')",
            [],
        )
        .unwrap();

        // The write should proceed despite the live CLI lease.
        put_from_cli_string(
            &conn,
            "notes/cli-owned",
            "---\ntitle: CLI owned\ntype: note\n---\nOK\n",
            None,
            None,
        )
        .unwrap();

        assert!(vault_root.join("notes").join("cli-owned.md").exists());
        assert_eq!(page_count(&conn, "notes/cli-owned"), 1);
    }

    #[cfg(all(unix, target_os = "linux"))]
    #[serial_test::serial]
    #[test]
    fn cli_put_proxies_through_live_serve_socket() {
        let _env_lock = env_mutation_lock()
            .lock()
            .unwrap_or_else(|err| err.into_inner());
        let runtime_root = secure_runtime_root();
        let _xdg = EnvVarGuard::set("XDG_RUNTIME_DIR", runtime_root.path().to_str().unwrap());
        let (_guard, _dir, db_path, conn, vault_root) = open_test_db_with_vault_guarded();
        let runtime = vault_sync::start_serve_runtime(db_path.clone()).unwrap();
        let cli_conn = Connection::open(&db_path).unwrap();
        cli_conn.busy_timeout(Duration::from_secs(2)).unwrap();

        let mut proxied = false;
        for _ in 0..20 {
            match put_from_cli_string(
                &cli_conn,
                "notes/live-owner",
                "---\ntitle: Live owner\ntype: note\n---\nProxied\n",
                None,
                None,
            ) {
                Ok(_) => {
                    proxied = true;
                    break;
                }
                Err(error) if error.to_string().contains("database is locked") => {
                    std::thread::sleep(Duration::from_millis(100));
                }
                Err(error) => panic!("unexpected live-serve proxy error: {error}"),
            }
        }
        assert!(
            proxied,
            "timed out waiting for live-serve proxy write to succeed"
        );

        assert_eq!(
            stdfs::read_to_string(vault_root.join("notes").join("live-owner.md")).unwrap(),
            "---\ntitle: Live owner\ntype: note\n---\nProxied\n"
        );
        assert_eq!(page_count(&conn, "notes/live-owner"), 1);
        let active_owner: Option<String> = conn
            .query_row(
                "SELECT active_lease_session_id FROM collections WHERE id = 1",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(active_owner.as_deref(), Some(runtime.session_id.as_str()));
        let ipc_path: String = conn
            .query_row(
                "SELECT ipc_path FROM serve_sessions WHERE session_id = ?1",
                [runtime.session_id.as_str()],
                |row| row.get(0),
            )
            .unwrap();
        drop(runtime);
        assert!(!Path::new(&ipc_path).exists());
        let post_runtime_sessions: i64 = conn
            .query_row("SELECT COUNT(*) FROM serve_sessions", [], |row| row.get(0))
            .unwrap();
        assert_eq!(post_runtime_sessions, 0);
    }

    #[cfg(all(unix, target_os = "linux"))]
    #[serial_test::serial]
    #[test]
    fn cli_put_refuses_same_uid_socket_spoof_with_pid_mismatch() {
        let _env_lock = env_mutation_lock()
            .lock()
            .unwrap_or_else(|err| err.into_inner());
        let runtime_root = secure_runtime_root();
        let socket_dir = runtime_root.path().join("quaid");
        let socket_path = socket_dir.join("serve-live.sock");
        let _xdg = EnvVarGuard::set("XDG_RUNTIME_DIR", runtime_root.path().to_str().unwrap());
        let (_guard, _dir, _db_path, conn, vault_root) = open_test_db_with_vault_guarded();
        let fake_server = spawn_fake_ipc_server(&socket_path, "serve-live");
        conn.execute(
            "INSERT INTO serve_sessions (session_id, pid, host, heartbeat_at, ipc_path)
             VALUES ('serve-live', 4321, 'serve-host', datetime('now'), ?1)",
            [socket_path.display().to_string()],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO collection_owners (collection_id, session_id) VALUES (1, 'serve-live')",
            [],
        )
        .unwrap();

        let error = put_from_cli_string(
            &conn,
            "notes/live-owner",
            "---\ntitle: Live owner\ntype: note\n---\nBlocked\n",
            None,
            None,
        )
        .unwrap_err();

        let text = error.to_string();
        assert!(text.contains("IpcPeerAuthFailedError"));
        assert!(text.contains("peer pid"));
        assert!(!vault_root.join("notes").join("live-owner.md").exists());
        assert_eq!(page_count(&conn, "notes/live-owner"), 0);
        drop(conn);
        fake_server.join().unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn cli_put_holds_short_lived_owner_lease_for_direct_write() {
        let (guard, _dir, db_path, conn, vault_root) = open_test_db_with_vault_guarded();
        reset_write_lock_blocker(&db_path);
        guard.set(PutTestHooks {
            block_inside_slug_lock: true,
            ..PutTestHooks::default()
        });

        let worker_db_path = db_path.clone();
        let handle = thread::spawn(move || {
            let worker_conn = Connection::open(worker_db_path).unwrap();
            worker_conn.busy_timeout(Duration::from_millis(0)).unwrap();
            put_from_cli_string(
                &worker_conn,
                "notes/offline-lease",
                "---\ntitle: Lease\ntype: note\n---\nHeld\n",
                None,
                None,
            )
        });

        wait_for_write_lock_entry(&db_path);

        let lease_snapshot: (i64, i64, Option<String>) = conn
            .query_row(
                "SELECT
                     (SELECT COUNT(*) FROM collection_owners),
                     (SELECT COUNT(*) FROM serve_sessions),
                     active_lease_session_id
                 FROM collections
                 WHERE id = 1",
                [],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .unwrap();
        assert_eq!(lease_snapshot.0, 1);
        assert_eq!(lease_snapshot.1, 1);
        assert!(lease_snapshot.2.is_some());

        release_write_lock_blocker(&db_path);
        handle.join().unwrap().unwrap();

        let released_snapshot: (i64, i64, Option<String>) = conn
            .query_row(
                "SELECT
                     (SELECT COUNT(*) FROM collection_owners),
                     (SELECT COUNT(*) FROM serve_sessions),
                     active_lease_session_id
                 FROM collections
                 WHERE id = 1",
                [],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .unwrap();
        assert_eq!(released_snapshot.0, 0);
        assert_eq!(released_snapshot.1, 0);
        assert!(released_snapshot.2.is_none());
        assert_eq!(
            stdfs::read_to_string(vault_root.join("notes").join("offline-lease.md")).unwrap(),
            "---\ntitle: Lease\ntype: note\n---\nHeld\n"
        );
    }
}
