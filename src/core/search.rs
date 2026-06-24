//! Hybrid retrieval composition: exact-slug short-circuit, FTS5 lexical
//! search, and semantic vector search merged into a single ranked result list.
//! Operates over the `HybridSearch` parameter struct so new filters (wing,
//! collection, namespace, superseded toggle, canonical-slug output) extend by
//! adding fields rather than spawning `_with_<flag>` siblings.
//!
//! See also: `fts` for the FTS5 layer this module composes, `inference` for
//! the vector backend, and `progressive` for the token-budget expansion step
//! that consumes these results.

use std::collections::{HashMap, HashSet};

use rusqlite::Connection;

use super::collections::{self, OpKind, SlugResolution};
use super::fts::{sanitize_fts_query, search_fts_tiered, FtsQuery};
use super::inference::{
    search_vec_canonical_with_namespace_filtered, search_vec_with_namespace_filtered,
};
use super::pages::{self, PageKey};
use super::types::{SearchError, SearchMergeStrategy, SearchResult};

/// Per-call options for [`hybrid_search`].
///
/// The `Default` impl gives every field its zero value (empty `&str`, `None`
/// for filters, `false` for booleans, `0` for `limit`). The canonical
/// call-site idiom names only the fields that diverge from `Default`:
///
/// ```ignore
/// HybridSearch {
///     query: "AI founder",
///     namespace: Some("test-ns"),
///     canonical: true,
///     limit: 10,
///     ..Default::default()
/// }
/// ```
///
/// New filters (e.g. `min_score`, additional facets) SHALL be added here as
/// new fields with sensible defaults rather than as a sibling
/// `_with_<flag>` function variant.
///
/// Field semantics:
/// - `query`: the natural-language input. `hybrid_search` short-circuits on
///   exact-slug matches and otherwise sanitizes before FTS5/vector search.
/// - `wing`: optional wing prefix filter.
/// - `collection`: optional collection-id filter.
/// - `namespace`: optional namespace filter; `Some("foo")` matches both
///   the `foo` namespace and the global `""` namespace.
/// - `include_superseded`: when `false`, hides pages with non-NULL
///   `superseded_by`.
/// - `include_quarantined`: when `false`, hides pages with non-NULL
///   `quarantined_at`. Quarantined pages must be fetched explicitly (e.g.
///   `memory_get`/raw access), never surfaced by retrieval.
/// - `canonical`: when `true`, results return slugs in
///   `<collection>::<slug>` form.
/// - `limit`: maximum number of rows to return after merge.
#[derive(Default, Clone)]
pub struct HybridSearch<'a> {
    /// Natural-language query string.
    pub query: &'a str,
    /// Optional wing prefix filter (e.g. `Some("people")`).
    pub wing: Option<&'a str>,
    /// Optional collection-id filter.
    pub collection: Option<i64>,
    /// Optional namespace filter; `Some("foo")` also matches the global namespace.
    pub namespace: Option<&'a str>,
    /// When `false`, hides pages whose `superseded_by` is non-NULL.
    pub include_superseded: bool,
    /// When `false`, hides pages whose `quarantined_at` is non-NULL.
    pub include_quarantined: bool,
    /// When `true`, results return slugs in `<collection>::<slug>` form.
    pub canonical: bool,
    /// Maximum number of rows to return after the merge step.
    pub limit: usize,
    /// Optional graph-expansion depth override. When `None`, the depth is
    /// read from `config.graph_depth` (default `0`). Set `Some(0)` to
    /// disable graph expansion for this invocation.
    pub hops: Option<u32>,
    /// Optional per-call relevance floor in `[0.0, 1.0]`. When `None`, the
    /// floor is read from `config.search.relevance_floor` (seeded `0.0` =
    /// disabled). Applied to the vector arm's raw cosine scores pre-merge
    /// and to the merged scores post-fusion; results below the floor are
    /// dropped even if fewer than `limit` rows remain.
    pub relevance_floor: Option<f64>,
    /// Optional per-call cap on rows retained per page in the merged
    /// results. When `None`, the cap is read from
    /// `config.search.max_chunks_per_doc_default` (seeded `0` = unlimited).
    pub max_chunks_per_doc: Option<usize>,
    /// Optional per-call MMR diversity parameter in `[0.0, 1.0]`. When
    /// `None`, the value is read from `config.search.mmr_lambda` (seeded
    /// `1.0` = identity: pure relevance ordering, no diversity penalty).
    /// `1.0` disables MMR; `0.0` is pure diversity selection.
    pub mmr_lambda: Option<f64>,
}

/// Hybrid search with exact-slug short-circuit, FTS5, and vector search.
///
/// At most `q.limit` results are returned. The limit is pushed into the FTS5
/// query and applied after the merge step to cap memory usage. See
/// [`HybridSearch`] for per-field documentation.
pub fn hybrid_search(
    conn: &Connection,
    q: HybridSearch<'_>,
) -> Result<Vec<SearchResult>, SearchError> {
    let trimmed = q.query.trim();
    if trimmed.is_empty() {
        return Ok(Vec::new());
    }

    if let Some(slug) = exact_slug_query(trimmed) {
        if let Some(result) = exact_slug_result_with_namespace(
            slug,
            q.wing,
            q.collection,
            q.namespace,
            q.include_superseded,
            q.include_quarantined,
            conn,
            q.canonical,
        )? {
            let mut results = vec![result];
            apply_graph_expansion(conn, &mut results, q.hops, q.collection)?;
            results.truncate(q.limit);
            return Ok(results);
        }
    }

    let fts_safe = sanitize_fts_query(trimmed);
    if !has_natural_language_terms(&fts_safe) {
        return Ok(Vec::new());
    }

    let fts_results = search_fts_tiered(
        conn,
        FtsQuery {
            query: &fts_safe,
            wing: q.wing,
            collection: q.collection,
            namespace: q.namespace,
            include_superseded: q.include_superseded,
            canonical: q.canonical,
            limit: q.limit,
        },
    )?;
    // The vector arm receives the caller's depth like FTS does, floored at
    // the historical k=10 and capped at 200 to bound the brute-force cosine
    // scan at high limits.
    let vec_k = q.limit.clamp(10, 200);
    let mut vec_results = if q.canonical {
        search_vec_canonical_with_namespace_filtered(
            trimmed,
            vec_k,
            q.wing,
            q.collection,
            q.namespace,
            q.include_superseded,
            conn,
        )?
    } else {
        search_vec_with_namespace_filtered(
            trimmed,
            vec_k,
            q.wing,
            q.collection,
            q.namespace,
            q.include_superseded,
            conn,
        )?
    };

    // Absolute cosine floor on the vector arm, applied to the raw cosine
    // scores before the per-arm normalization in the merge step. The seeded
    // default `0.0` is identity behaviour: no hit is dropped.
    let relevance_floor = match q.relevance_floor {
        Some(floor) => floor.clamp(0.0, 1.0),
        None => configured_relevance_floor(conn)?,
    };
    if relevance_floor > 0.0 {
        vec_results.retain(|result| result.score >= relevance_floor);
    }

    let merged = match read_merge_strategy(conn)? {
        SearchMergeStrategy::SetUnion => merge_set_union(&fts_results, &vec_results),
        SearchMergeStrategy::Rrf => merge_rrf(&fts_results, &vec_results),
    };

    // Post-fusion quality passes (dedup → floor), each an identity no-op at
    // its seeded default. The floor runs before graph expansion so a
    // below-floor noise hit is never used as an expansion seed.
    let max_chunks_per_doc = match q.max_chunks_per_doc {
        Some(value) => value,
        None => configured_max_chunks_per_doc(conn)?,
    };
    let merged = dedup_chunks_per_page(merged, max_chunks_per_doc);
    // Cross-reference boost runs between dedup and the floor so a co-cited
    // candidate can be rescued above the floor, and so the floor and MMR see
    // the post-boost score. The pass short-circuits (no `links` lookups) when
    // the configured weight is `0.0` — the seeded identity default.
    let (cross_ref_weight, cross_ref_cap) = configured_cross_ref(conn)?;
    let merged = compute_cross_ref_boost(conn, merged, cross_ref_weight, cross_ref_cap)?;
    let mut merged = filter_below_floor(merged, relevance_floor);

    apply_graph_expansion(conn, &mut merged, q.hops, q.collection)?;

    // MMR diversity rerank is the final post-fusion pass. It reads the
    // configured `search.mmr_lambda` (seeded `1.0` = identity: pure relevance
    // ordering reproduced bytewise) unless overridden per call.
    let mmr_lambda = match q.mmr_lambda {
        Some(value) => value.clamp(0.0, 1.0),
        None => configured_mmr_lambda(conn)?,
    };
    let mut merged = apply_mmr(conn, merged, mmr_lambda, q.limit);

    merged.truncate(q.limit);

    // Opt-in extractive reranker: rewrites each result's preview to the most
    // query-relevant sentence span. Gated behind `search.rerank_extractive`
    // (seeded `false`); a full no-op at the default. Applied after truncation
    // so the budget is only spent on rows that survive to the output.
    let merged = apply_extractive_rerank(conn, trimmed, merged)?;
    Ok(merged)
}

/// Read the configured post-fusion relevance floor
/// (`config.search.relevance_floor`, seeded `0.0` = disabled), clamped into
/// the valid `[0.0, 1.0]` range.
pub fn configured_relevance_floor(conn: &Connection) -> Result<f64, SearchError> {
    Ok(read_config_f64(conn, "search.relevance_floor", 0.0)?.clamp(0.0, 1.0))
}

/// Read the configured per-page result cap
/// (`config.search.max_chunks_per_doc_default`, seeded `0` = unlimited).
pub fn configured_max_chunks_per_doc(conn: &Connection) -> Result<usize, SearchError> {
    read_config_usize(conn, "search.max_chunks_per_doc_default", 0)
}

/// Read the configured MMR diversity parameter
/// (`config.search.mmr_lambda`, seeded `1.0` = identity / disabled), clamped
/// into the valid `[0.0, 1.0]` range.
pub fn configured_mmr_lambda(conn: &Connection) -> Result<f64, SearchError> {
    Ok(read_config_f64(conn, "search.mmr_lambda", 1.0)?.clamp(0.0, 1.0))
}

/// Read the configured cross-reference boost weight and cap
/// (`config.search.cross_ref_boost_weight`, seeded `0.0` = disabled, and
/// `config.search.cross_ref_boost_cap`, seeded `0.15`), each clamped into the
/// valid `[0.0, 1.0]` range.
pub fn configured_cross_ref(conn: &Connection) -> Result<(f64, f64), SearchError> {
    let weight = read_config_f64(conn, "search.cross_ref_boost_weight", 0.0)?.clamp(0.0, 1.0);
    let cap = read_config_f64(conn, "search.cross_ref_boost_cap", 0.15)?.clamp(0.0, 1.0);
    Ok((weight, cap))
}

/// Configuration for the opt-in extractive reranker.
#[derive(Debug, Clone, Copy)]
pub struct ExtractiveConfig {
    /// Whether the extractive pass is enabled (`search.rerank_extractive`,
    /// seeded `false`).
    pub enabled: bool,
    /// Number of contiguous sentences to select (`search.rerank_extractive_top_n`,
    /// seeded `3`).
    pub top_n: usize,
    /// Per-chunk wall-clock budget in milliseconds
    /// (`search.rerank_extractive_budget_ms`, seeded `10`).
    pub budget_ms: u64,
}

/// Read the extractive-rerank configuration. Disabled (`enabled = false`) is
/// the seeded identity default and a full no-op.
pub fn configured_extractive(conn: &Connection) -> Result<ExtractiveConfig, SearchError> {
    let enabled = read_config_bool(conn, "search.rerank_extractive", false)?;
    let top_n = read_config_usize(conn, "search.rerank_extractive_top_n", 3)?;
    let budget_ms = read_config_u64(conn, "search.rerank_extractive_budget_ms", 10)?;
    Ok(ExtractiveConfig {
        enabled,
        top_n,
        budget_ms,
    })
}

/// Collapse multi-chunk hits from the same page down to at most
/// `max_per_page` representatives, accumulating the number of collapsed
/// siblings into the strongest surviving row's `dedup_collapsed_count`.
///
/// Candidates are expected in score-descending order (the order every
/// retrieval arm and merge strategy produces); relative order is preserved.
/// `max_per_page == 0` means unlimited and returns the input unchanged —
/// the seeded identity default.
pub fn dedup_chunks_per_page(
    candidates: Vec<SearchResult>,
    max_per_page: usize,
) -> Vec<SearchResult> {
    if max_per_page == 0 {
        return candidates;
    }

    let mut kept: Vec<SearchResult> = Vec::with_capacity(candidates.len());
    let mut kept_by_page: HashMap<String, Vec<usize>> = HashMap::new();
    for candidate in candidates {
        let entry = kept_by_page.entry(candidate.slug.clone()).or_default();
        if entry.len() < max_per_page {
            entry.push(kept.len());
            kept.push(candidate);
        } else {
            let strongest = entry
                .iter()
                .copied()
                .max_by(|left, right| kept[*left].score.total_cmp(&kept[*right].score))
                .unwrap_or(entry[0]);
            kept[strongest].dedup_collapsed_count += 1 + candidate.dedup_collapsed_count;
        }
    }
    kept
}

/// Drop candidates whose score falls below `floor`, even if fewer than the
/// requested number of results remain ("fewer-than-k" contract).
///
/// `floor <= 0.0` disables filtering and returns the input unchanged — the
/// seeded identity default. Scores exactly at the floor are kept.
pub fn filter_below_floor(candidates: Vec<SearchResult>, floor: f64) -> Vec<SearchResult> {
    if floor <= 0.0 {
        return candidates;
    }
    candidates
        .into_iter()
        .filter(|candidate| candidate.score >= floor)
        .collect()
}

/// Add an additive, capped cross-reference boost to each candidate's `score`
/// based on active `links` edges incoming from other members of the working
/// set (depth-1 co-citation).
///
/// For each candidate `c`,
/// `boost(c) = min(weight · Σ edge_weight(s → c), cap)` summed over candidates
/// `s` in the working set that link to `c` through a currently-valid edge. The
/// boost is folded into `score` and surfaced on `cross_ref_boost`.
///
/// `weight <= 0.0` short-circuits the pass entirely — no `links` lookups are
/// performed and the input is returned unchanged (the seeded identity
/// default). The pass degrades gracefully when the graph is sparse or absent:
/// an empty edge set leaves every candidate untouched. Candidate ordering is
/// not changed here; the caller re-sorts (via the floor/MMR passes) so the
/// boost can influence downstream ordering.
pub fn compute_cross_ref_boost(
    conn: &Connection,
    mut candidates: Vec<SearchResult>,
    weight: f64,
    cap: f64,
) -> Result<Vec<SearchResult>, SearchError> {
    if weight <= 0.0 || candidates.len() < 2 {
        return Ok(candidates);
    }

    // Resolve each candidate slug to its page id. Slugs that do not resolve
    // (e.g. graph-expansion stubs) simply do not participate in co-citation.
    let mut page_id_by_index: Vec<Option<i64>> = Vec::with_capacity(candidates.len());
    let mut index_by_page_id: HashMap<i64, usize> = HashMap::new();
    for (index, candidate) in candidates.iter().enumerate() {
        let page_id = resolve_candidate_page_id(conn, &candidate.slug);
        if let Some(id) = page_id {
            index_by_page_id.insert(id, index);
        }
        page_id_by_index.push(page_id);
    }

    let candidate_ids: Vec<i64> = page_id_by_index.iter().filter_map(|id| *id).collect();
    if candidate_ids.len() < 2 {
        return Ok(candidates);
    }

    // Single indexed query: active edges whose both endpoints are members of
    // the working set. Endpoint ids are trusted i64s read from `pages.id`, so
    // they are inlined as integer literals (injection-safe by construction)
    // rather than bound one parameter each, mirroring the vector-search path.
    use std::fmt::Write as _;
    let mut id_list = String::new();
    for (position, id) in candidate_ids.iter().enumerate() {
        if position > 0 {
            id_list.push(',');
        }
        let _ = write!(id_list, "{id}");
    }
    let sql = format!(
        "SELECT from_page_id, to_page_id, edge_weight \
         FROM links \
         WHERE from_page_id IN ({id_list}) \
           AND to_page_id IN ({id_list}) \
           AND from_page_id != to_page_id \
           AND (valid_from IS NULL OR valid_from <= date('now')) \
           AND (valid_until IS NULL OR valid_until >= date('now'))"
    );

    // Accumulate the raw (pre-cap) boost per target index.
    let mut raw_boost: HashMap<usize, f64> = HashMap::new();
    {
        let mut stmt = conn.prepare(&sql)?;
        let rows = stmt.query_map([], |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, i64>(1)?,
                row.get::<_, f64>(2)?,
            ))
        })?;
        for row in rows {
            let (from_id, to_id, edge_weight) = row?;
            // Both endpoints are guaranteed in the working set by the SQL
            // filter; skip self-references defensively.
            if from_id == to_id {
                continue;
            }
            if let Some(&target_index) = index_by_page_id.get(&to_id) {
                *raw_boost.entry(target_index).or_insert(0.0) += edge_weight;
            }
        }
    }

    // `cap` is already clamped into `[0.0, 1.0]` by `configured_cross_ref`, so
    // the lower bound never exceeds the upper bound here.
    let cap = cap.max(0.0);
    let mut any_boost = false;
    for (index, candidate) in candidates.iter_mut().enumerate() {
        if let Some(sum) = raw_boost.get(&index) {
            let boost = (weight * sum).clamp(0.0, cap);
            if boost > 0.0 {
                candidate.cross_ref_boost = boost as f32;
                candidate.score += boost;
                any_boost = true;
            }
        }
    }

    // Re-sort only when a boost actually moved a score, so the floor, graph
    // expansion, and an identity MMR pass all observe the post-boost ranking.
    // When no boost applied (sparse/empty graph) the list is left untouched,
    // preserving the bytewise identity path.
    if any_boost {
        candidates.sort_by(|left, right| {
            right
                .score
                .total_cmp(&left.score)
                .then_with(|| left.slug.cmp(&right.slug))
        });
    }

    Ok(candidates)
}

/// Resolve a (possibly canonical) candidate slug to its page id for
/// cross-reference scoring. Returns `None` when the slug cannot be resolved,
/// which simply excludes the candidate from co-citation rather than erroring.
fn resolve_candidate_page_id(conn: &Connection, slug: &str) -> Option<i64> {
    let (collection_id, resolved_slug) = resolve_slug_key(conn, slug)?;
    pages::resolve_optional(
        conn,
        &PageKey {
            collection_id,
            namespace: None,
            slug: &resolved_slug,
        },
    )
    .ok()
    .flatten()
}

/// Apply Maximal Marginal Relevance (MMR) as the final greedy reranking pass
/// over a post-fusion candidate list.
///
/// For each candidate `c`,
/// `mmr(c) = λ · fused_score(c) − (1 − λ) · max_{s ∈ selected} cosine(c, s)`.
/// Selection is greedy by maximum MMR score until `k` are chosen or the
/// candidates are exhausted. Ties break deterministically on
/// `(fused_score desc, page_id asc, slug asc)`; the slug is the final
/// tie-break (a stable proxy for `chunk_id asc` since rows carry no chunk id)
/// so identical inputs always yield identical orderings.
///
/// `lambda >= 1.0` is the identity case: it reproduces the pre-change
/// relevance ordering bytewise (the input is already sorted by
/// `score desc, slug asc` from the merge step), so the input is returned
/// untouched and `mmr_score` is left at its inactive default. Candidates
/// without a stored embedding incur zero diversity penalty.
pub fn apply_mmr(
    conn: &Connection,
    candidates: Vec<SearchResult>,
    lambda: f64,
    k: usize,
) -> Vec<SearchResult> {
    // Identity: λ == 1.0 is pure relevance ordering. The merge/expansion
    // steps already produced `score desc, slug asc`; leaving the list and the
    // `mmr_score` field untouched guarantees bytewise-identical output to the
    // pre-MMR pipeline.
    if lambda >= 1.0 || candidates.len() < 2 {
        return candidates;
    }

    let limit = if k == 0 {
        candidates.len()
    } else {
        k.min(candidates.len())
    };

    // Fetch each candidate's representative embedding and page id once. Missing
    // vectors (failed-to-embed chunks, FTS-only hits) map to `None` and
    // contribute a zero diversity penalty. The page id feeds the deterministic
    // tie-break; `i64::MAX` sorts unresolved rows last.
    let vectors: Vec<Option<Vec<f32>>> = candidates
        .iter()
        .map(|candidate| candidate_embedding(conn, &candidate.slug))
        .collect();
    let page_ids: Vec<i64> = candidates
        .iter()
        .map(|candidate| resolve_candidate_page_id(conn, &candidate.slug).unwrap_or(i64::MAX))
        .collect();

    let mut remaining: Vec<usize> = (0..candidates.len()).collect();
    let mut selected: Vec<usize> = Vec::with_capacity(limit);
    let mut mmr_scores: Vec<f32> = vec![0.0; candidates.len()];

    while !remaining.is_empty() && selected.len() < limit {
        let mut best_remaining_pos = 0usize;
        let mut best_mmr = f64::NEG_INFINITY;
        let mut best_index = remaining[0];

        for (position, &index) in remaining.iter().enumerate() {
            let diversity = match &vectors[index] {
                Some(vec_c) => selected
                    .iter()
                    .filter_map(|&s| vectors[s].as_ref())
                    .map(|vec_s| cosine_similarity_f32(vec_c, vec_s))
                    .fold(0.0_f64, f64::max),
                None => 0.0,
            };
            let mmr = lambda * candidates[index].score - (1.0 - lambda) * diversity;

            let replace = if mmr > best_mmr {
                true
            } else if mmr < best_mmr {
                false
            } else {
                // Tie on MMR → (fused_score desc, page_id asc, slug asc).
                match candidates[index].score.total_cmp(&candidates[best_index].score) {
                    std::cmp::Ordering::Greater => true,
                    std::cmp::Ordering::Less => false,
                    std::cmp::Ordering::Equal => match page_ids[index].cmp(&page_ids[best_index]) {
                        std::cmp::Ordering::Less => true,
                        std::cmp::Ordering::Greater => false,
                        std::cmp::Ordering::Equal => {
                            candidates[index].slug < candidates[best_index].slug
                        }
                    },
                }
            };

            if replace {
                best_mmr = mmr;
                best_remaining_pos = position;
                best_index = index;
            }
        }

        mmr_scores[best_index] = best_mmr as f32;
        selected.push(best_index);
        remaining.remove(best_remaining_pos);
    }

    // Reassemble selected candidates in selection order, recording the MMR
    // score at the moment of selection. Candidates beyond `limit` are dropped
    // (the caller truncates to `limit` regardless).
    let mut by_index: HashMap<usize, SearchResult> = candidates.into_iter().enumerate().collect();
    let mut out: Vec<SearchResult> = Vec::with_capacity(selected.len());
    for index in selected {
        if let Some(mut candidate) = by_index.remove(&index) {
            candidate.mmr_score = Some(mmr_scores[index]);
            out.push(candidate);
        }
    }
    out
}

/// Cosine similarity between two `f32` vectors, reusing the same numerically
/// stable f64-accumulation primitive as the conversation supersede path.
/// Returns `0.0` for mismatched-length or empty inputs.
fn cosine_similarity_f32(left: &[f32], right: &[f32]) -> f64 {
    if left.len() != right.len() || left.is_empty() {
        return 0.0;
    }
    let mut dot = 0.0f64;
    let mut left_norm = 0.0f64;
    let mut right_norm = 0.0f64;
    for (l, r) in left.iter().zip(right.iter()) {
        let l = *l as f64;
        let r = *r as f64;
        dot += l * r;
        left_norm += l * l;
        right_norm += r * r;
    }
    if left_norm == 0.0 || right_norm == 0.0 {
        0.0
    } else {
        dot / (left_norm.sqrt() * right_norm.sqrt())
    }
}

/// Read a representative embedding for a candidate page (the first stored
/// chunk under the active model), decoded from the `vec0` virtual table.
/// Returns `None` when the slug cannot be resolved or no embedding exists, so
/// MMR can treat the candidate as a zero-penalty fall-through.
fn candidate_embedding(conn: &Connection, slug: &str) -> Option<Vec<f32>> {
    let page_id = resolve_candidate_page_id(conn, slug)?;
    let (model_name, vec_table) = active_model(conn).ok()?;
    if !is_safe_vec_table(&vec_table) {
        return None;
    }
    let sql = format!(
        "SELECT vt.embedding FROM page_embeddings pe \
         JOIN {vec_table} vt ON vt.rowid = pe.vec_rowid \
         WHERE pe.page_id = ?1 AND pe.model = ?2 \
         ORDER BY pe.chunk_index ASC LIMIT 1"
    );
    let blob: Vec<u8> = conn
        .query_row(&sql, rusqlite::params![page_id, model_name], |row| {
            row.get(0)
        })
        .ok()?;
    blob_to_embedding(&blob)
}

/// Decode a little-endian `f32` blob (the `vec0` storage format) back into a
/// vector. Returns `None` when the byte length is not a multiple of 4.
fn blob_to_embedding(blob: &[u8]) -> Option<Vec<f32>> {
    if blob.is_empty() || !blob.len().is_multiple_of(4) {
        return None;
    }
    let mut out = Vec::with_capacity(blob.len() / 4);
    for chunk in blob.chunks_exact(4) {
        let bytes = [chunk[0], chunk[1], chunk[2], chunk[3]];
        out.push(f32::from_le_bytes(bytes));
    }
    Some(out)
}

/// Read the active model name and its backing `vec0` table from
/// `embedding_models`. Returns an error when no active model is configured.
fn active_model(conn: &Connection) -> Result<(String, String), SearchError> {
    conn.query_row(
        "SELECT name, vec_table FROM embedding_models WHERE active = 1 LIMIT 1",
        [],
        |row| Ok((row.get(0)?, row.get(1)?)),
    )
    .map_err(|err| match err {
        rusqlite::Error::QueryReturnedNoRows => SearchError::Internal {
            message: "no active embedding model configured".to_owned(),
        },
        other => SearchError::from(other),
    })
}

/// Guard against SQL injection through the `vec_table` identifier read from
/// config: only ASCII alphanumerics and underscores are permitted.
fn is_safe_vec_table(name: &str) -> bool {
    !name.is_empty()
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_')
}

/// Apply the opt-in extractive reranker to each result's preview when
/// `config.search.rerank_extractive` is enabled. For each candidate, the
/// representative chunk text is segmented into sentences and the most
/// query-relevant contiguous span replaces the result's `summary` (the
/// retrieval preview). Disabled is the seeded identity default and a full
/// no-op — the input is returned untouched without embedding the query.
///
/// Embedding failures, short chunks, missing chunk text, or budget timeouts
/// leave a candidate's preview unchanged; no candidate is ever dropped or
/// reordered by this pass.
pub fn apply_extractive_rerank(
    conn: &Connection,
    query: &str,
    mut candidates: Vec<SearchResult>,
) -> Result<Vec<SearchResult>, SearchError> {
    let cfg = configured_extractive(conn)?;
    if !cfg.enabled || cfg.top_n == 0 || candidates.is_empty() {
        return Ok(candidates);
    }

    // Embed the query once. If the query cannot be embedded, the pass no-ops
    // rather than erroring (retrieval already succeeded).
    let Ok(query_vec) = crate::core::inference::embed_query(query) else {
        return Ok(candidates);
    };

    for candidate in candidates.iter_mut() {
        let Some(chunk_text) = representative_chunk_text(conn, &candidate.slug) else {
            continue;
        };
        let outcome = crate::core::rerank::extractive_rerank(
            &chunk_text,
            &query_vec,
            cfg.top_n,
            cfg.budget_ms,
            |sentence| crate::core::inference::embed(sentence).ok(),
        );
        if let crate::core::rerank::RerankOutcome::Selected(span) = outcome {
            candidate.summary = span;
        }
    }

    Ok(candidates)
}

/// Read the representative chunk text for a candidate page (the first stored
/// chunk under the active model), used as the source text for extractive
/// reranking. Returns `None` when the slug cannot be resolved or no chunk
/// exists.
fn representative_chunk_text(conn: &Connection, slug: &str) -> Option<String> {
    let page_id = resolve_candidate_page_id(conn, slug)?;
    let (model_name, _vec_table) = active_model(conn).ok()?;
    conn.query_row(
        "SELECT chunk_text FROM page_embeddings \
         WHERE page_id = ?1 AND model = ?2 \
         ORDER BY chunk_index ASC LIMIT 1",
        rusqlite::params![page_id, model_name],
        |row| row.get::<_, String>(0),
    )
    .ok()
}

/// Bounded outbound graph expansion knobs.
#[derive(Debug, Clone, Copy)]
pub struct GraphExpansionConfig {
    /// Maximum BFS depth to walk from each candidate.
    pub depth: u32,
    /// Multiplicative score penalty applied per hop (`decay^hops`).
    pub distance_decay: f64,
    /// Hard cap on how many new candidates expansion may add.
    pub max_added: usize,
}

impl GraphExpansionConfig {
    /// Read all graph-expansion knobs from the `config` table, falling back
    /// to the v10-seeded defaults when a key is missing or unparseable.
    pub fn from_config(conn: &Connection) -> Result<Self, SearchError> {
        Ok(Self {
            depth: read_config_u32(conn, "graph_depth", 0)?,
            distance_decay: read_config_f64(conn, "graph_distance_decay", 0.5)?,
            max_added: read_config_usize(conn, "graph_expansion_max", 50)?,
        })
    }
}

fn read_config_u32(conn: &Connection, key: &str, default: u32) -> Result<u32, SearchError> {
    let value: Option<String> =
        match conn.query_row("SELECT value FROM config WHERE key = ?1", [key], |row| {
            row.get(0)
        }) {
            Ok(v) => Some(v),
            Err(rusqlite::Error::QueryReturnedNoRows) => None,
            Err(err) => return Err(SearchError::from(err)),
        };
    Ok(value.and_then(|v| v.parse().ok()).unwrap_or(default))
}

fn read_config_f64(conn: &Connection, key: &str, default: f64) -> Result<f64, SearchError> {
    let value: Option<String> =
        match conn.query_row("SELECT value FROM config WHERE key = ?1", [key], |row| {
            row.get(0)
        }) {
            Ok(v) => Some(v),
            Err(rusqlite::Error::QueryReturnedNoRows) => None,
            Err(err) => return Err(SearchError::from(err)),
        };
    Ok(value.and_then(|v| v.parse().ok()).unwrap_or(default))
}

fn read_config_usize(conn: &Connection, key: &str, default: usize) -> Result<usize, SearchError> {
    let value: Option<String> =
        match conn.query_row("SELECT value FROM config WHERE key = ?1", [key], |row| {
            row.get(0)
        }) {
            Ok(v) => Some(v),
            Err(rusqlite::Error::QueryReturnedNoRows) => None,
            Err(err) => return Err(SearchError::from(err)),
        };
    Ok(value.and_then(|v| v.parse().ok()).unwrap_or(default))
}

fn read_config_u64(conn: &Connection, key: &str, default: u64) -> Result<u64, SearchError> {
    let value: Option<String> =
        match conn.query_row("SELECT value FROM config WHERE key = ?1", [key], |row| {
            row.get(0)
        }) {
            Ok(v) => Some(v),
            Err(rusqlite::Error::QueryReturnedNoRows) => None,
            Err(err) => return Err(SearchError::from(err)),
        };
    Ok(value.and_then(|v| v.parse().ok()).unwrap_or(default))
}

fn read_config_bool(conn: &Connection, key: &str, default: bool) -> Result<bool, SearchError> {
    let value: Option<String> =
        match conn.query_row("SELECT value FROM config WHERE key = ?1", [key], |row| {
            row.get(0)
        }) {
            Ok(v) => Some(v),
            Err(rusqlite::Error::QueryReturnedNoRows) => None,
            Err(err) => return Err(SearchError::from(err)),
        };
    Ok(value
        .map(|v| matches!(v.trim().to_ascii_lowercase().as_str(), "true" | "1" | "yes"))
        .unwrap_or(default))
}

/// Apply bounded outbound graph expansion to a ranked candidate list in
/// place. The effective depth is `hops_override` when provided, otherwise
/// `config.graph_depth`. When the effective depth is `0`, the function
/// returns immediately and `merged` is left untouched (baseline behaviour).
///
/// Newly discovered candidates are appended in score-descending order and
/// then the full list is re-sorted by score so graph hits compete with the
/// original top-K.
fn apply_graph_expansion(
    conn: &Connection,
    merged: &mut Vec<SearchResult>,
    hops_override: Option<u32>,
    collection_filter: Option<i64>,
) -> Result<(), SearchError> {
    if merged.is_empty() {
        return Ok(());
    }
    let cfg = GraphExpansionConfig::from_config(conn)?;
    let depth = hops_override.unwrap_or(cfg.depth);
    if depth == 0 {
        return Ok(());
    }
    let added = expand_graph(
        conn,
        merged,
        depth,
        cfg.max_added,
        cfg.distance_decay,
        collection_filter,
    )?;
    if added.is_empty() {
        return Ok(());
    }
    merged.extend(added);
    merged.sort_by(|left, right| {
        right
            .score
            .total_cmp(&left.score)
            .then_with(|| left.slug.cmp(&right.slug))
    });
    Ok(())
}

/// Hard safety cap on the cumulative nodes visited during graph expansion,
/// regardless of `graph_expansion_max`.
const GRAPH_EXPANSION_MAX_NODES: usize = 1000;

/// Walk outbound `links` from each candidate up to `depth` hops, scoring
/// each newly discovered slug as `parent_score * edge_weight * decay^hops`.
///
/// Slugs already present in `candidates` are not re-added. The expansion is
/// capped at `max_added` new entries and `GRAPH_EXPANSION_MAX_NODES`
/// cumulative visited nodes. When a slug is reachable through several
/// parents, the highest score wins.
pub fn expand_graph(
    conn: &Connection,
    candidates: &[SearchResult],
    depth: u32,
    max_added: usize,
    distance_decay: f64,
    collection_filter: Option<i64>,
) -> Result<Vec<SearchResult>, SearchError> {
    if candidates.is_empty() || depth == 0 || max_added == 0 {
        return Ok(Vec::new());
    }

    let canonical = candidates
        .first()
        .map(|c| c.slug.contains("::"))
        .unwrap_or(false);

    let initial_slugs: HashSet<String> = candidates.iter().map(|c| c.slug.clone()).collect();
    let mut best: HashMap<String, SearchResult> = HashMap::new();
    let mut visited: HashSet<String> = initial_slugs.clone();
    let mut total_visited: usize = candidates.len();

    let target_slug_expr = if canonical {
        "c2.name || '::' || p2.slug"
    } else {
        "p2.slug"
    };
    let collection_join = if canonical {
        " JOIN collections c2 ON c2.id = p2.collection_id"
    } else {
        ""
    };
    let collection_clause = if collection_filter.is_some() {
        " AND p2.collection_id = ?3"
    } else {
        ""
    };
    let sql = format!(
        "SELECT {target_slug_expr}, p2.title, p2.summary, p2.wing, l.edge_weight \
         FROM links l \
         JOIN pages p1 ON l.from_page_id = p1.id \
         JOIN pages p2 ON l.to_page_id = p2.id{collection_join} \
         WHERE p1.collection_id = ?1 AND p1.slug = ?2 \
           AND p2.superseded_by IS NULL \
           AND p2.quarantined_at IS NULL \
           AND (l.valid_from IS NULL OR l.valid_from <= date('now')) \
           AND (l.valid_until IS NULL OR l.valid_until >= date('now'))\
           {collection_clause}"
    );

    let mut frontier: Vec<(String, f64)> = candidates
        .iter()
        .map(|c| (c.slug.clone(), c.score))
        .collect();

    'outer: for hop in 1..=depth {
        if frontier.is_empty() {
            break;
        }
        let hop_decay = distance_decay.powi(hop as i32);
        let mut next_frontier: Vec<(String, f64)> = Vec::new();

        for (parent_slug, parent_score) in &frontier {
            let Some((collection_id, resolved_slug)) = resolve_slug_key(conn, parent_slug) else {
                continue;
            };
            let mut stmt = conn.prepare_cached(&sql)?;
            let row_fn = |row: &rusqlite::Row<'_>| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, f64>(4)?,
                ))
            };
            let rows: Vec<(String, String, String, String, f64)> =
                if let Some(cid) = collection_filter {
                    stmt.query_map(rusqlite::params![collection_id, resolved_slug, cid], row_fn)?
                        .collect::<rusqlite::Result<Vec<_>>>()?
                } else {
                    stmt.query_map(rusqlite::params![collection_id, resolved_slug], row_fn)?
                        .collect::<rusqlite::Result<Vec<_>>>()?
                };
            for row in rows {
                let (target_slug, title, summary, wing, edge_weight) = row;
                if initial_slugs.contains(&target_slug) {
                    continue;
                }
                let score = parent_score * edge_weight * hop_decay;
                best.entry(target_slug.clone())
                    .and_modify(|existing| {
                        if score > existing.score {
                            existing.score = score;
                        }
                    })
                    .or_insert(SearchResult {
                        slug: target_slug.clone(),
                        title,
                        summary,
                        score,
                        wing,
                        ..Default::default()
                    });
                if visited.insert(target_slug.clone()) {
                    total_visited += 1;
                    if total_visited >= GRAPH_EXPANSION_MAX_NODES {
                        break 'outer;
                    }
                    next_frontier.push((target_slug, score));
                }
            }
        }

        frontier = next_frontier;
    }

    let mut additions: Vec<SearchResult> = best.into_values().collect();
    additions.sort_by(|left, right| {
        right
            .score
            .total_cmp(&left.score)
            .then_with(|| left.slug.cmp(&right.slug))
    });
    additions.truncate(max_added);
    Ok(additions)
}

fn resolve_slug_key(conn: &Connection, slug: &str) -> Option<(i64, String)> {
    match collections::parse_slug(conn, slug, OpKind::Read).ok()? {
        SlugResolution::Resolved {
            collection_id,
            slug,
            ..
        } => Some((collection_id, slug)),
        SlugResolution::NotFound { .. } | SlugResolution::Ambiguous { .. } => None,
    }
}

fn has_natural_language_terms(fts_safe: &str) -> bool {
    const QUOTED_FTS5_KEYWORDS: &[&str] = &["\"AND\"", "\"OR\"", "\"NOT\"", "\"NEAR\""];

    fts_safe
        .split_whitespace()
        .any(|token| !QUOTED_FTS5_KEYWORDS.contains(&token))
}

/// Reads the configured hybrid-search merge strategy.
pub fn read_merge_strategy(conn: &Connection) -> Result<SearchMergeStrategy, SearchError> {
    let value = conn.query_row(
        "SELECT value FROM config WHERE key = 'search_merge_strategy'",
        [],
        |row| row.get::<_, String>(0),
    );

    match value {
        Ok(value) => Ok(SearchMergeStrategy::from_config(&value)),
        Err(rusqlite::Error::QueryReturnedNoRows) => Ok(SearchMergeStrategy::SetUnion),
        Err(err) => Err(SearchError::from(err)),
    }
}

fn exact_slug_query(query: &str) -> Option<&str> {
    let stripped = query
        .strip_prefix("[[")
        .and_then(|value| value.strip_suffix("]]"))
        .unwrap_or(query);
    let trimmed = stripped.trim();

    if trimmed.is_empty() || trimmed.contains(char::is_whitespace) {
        None
    } else {
        Some(trimmed)
    }
}

#[cfg(test)]
fn exact_slug_result(
    slug: &str,
    wing: Option<&str>,
    collection_filter: Option<i64>,
    include_superseded: bool,
    conn: &Connection,
    canonical_slug: bool,
) -> Result<Option<SearchResult>, SearchError> {
    exact_slug_result_with_namespace(
        slug,
        wing,
        collection_filter,
        None,
        include_superseded,
        false,
        conn,
        canonical_slug,
    )
}

#[expect(
    clippy::too_many_arguments,
    reason = "internal exact-slug helper threads the full HybridSearch filter set; a struct here would duplicate HybridSearch"
)]
fn exact_slug_result_with_namespace(
    slug: &str,
    wing: Option<&str>,
    collection_filter: Option<i64>,
    namespace_filter: Option<&str>,
    include_superseded: bool,
    include_quarantined: bool,
    conn: &Connection,
    canonical_slug: bool,
) -> Result<Option<SearchResult>, SearchError> {
    if canonical_slug {
        return exact_slug_result_canonical_with_namespace(
            slug,
            wing,
            collection_filter,
            namespace_filter,
            include_superseded,
            include_quarantined,
            conn,
        );
    }

    let mut query = String::from("SELECT slug, title, summary, wing FROM pages WHERE slug = ?1");
    let mut params: Vec<Box<dyn rusqlite::types::ToSql>> = vec![Box::new(slug.to_owned())];

    if let Some(wing) = wing {
        query.push_str(" AND wing = ?");
        query.push_str(&(params.len() + 1).to_string());
        params.push(Box::new(wing.to_owned()));
    }
    if let Some(collection_id) = collection_filter {
        query.push_str(" AND collection_id = ?");
        query.push_str(&(params.len() + 1).to_string());
        params.push(Box::new(collection_id));
    }
    if let Some(namespace) = namespace_filter {
        if namespace.is_empty() {
            query.push_str(" AND namespace = ?");
            query.push_str(&(params.len() + 1).to_string());
            params.push(Box::new(String::new()));
        } else {
            query.push_str(" AND (namespace = ?");
            query.push_str(&(params.len() + 1).to_string());
            query.push_str(" OR namespace = '')");
            params.push(Box::new(namespace.to_owned()));
        }
    }
    if !include_superseded {
        query.push_str(" AND superseded_by IS NULL");
    }
    if !include_quarantined {
        query.push_str(" AND quarantined_at IS NULL");
    }
    if let Some(namespace) = namespace_filter.filter(|namespace| !namespace.is_empty()) {
        query.push_str(" ORDER BY CASE WHEN namespace = ?");
        query.push_str(&(params.len() + 1).to_string());
        query.push_str(" THEN 0 ELSE 1 END");
        params.push(Box::new(namespace.to_owned()));
    }
    query.push_str(" LIMIT 1");

    let param_refs: Vec<&dyn rusqlite::types::ToSql> = params.iter().map(|p| p.as_ref()).collect();
    let result = conn.query_row(&query, param_refs.as_slice(), |row| {
        Ok(SearchResult {
            slug: row.get(0)?,
            title: row.get(1)?,
            summary: row.get(2)?,
            score: 1.0,
            wing: row.get(3)?,
            ..Default::default()
        })
    });

    match result {
        Ok(result) => Ok(Some(result)),
        Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
        Err(err) => Err(SearchError::from(err)),
    }
}

#[cfg(test)]
fn exact_slug_result_canonical(
    slug: &str,
    wing: Option<&str>,
    collection_filter: Option<i64>,
    include_superseded: bool,
    conn: &Connection,
) -> Result<Option<SearchResult>, SearchError> {
    exact_slug_result_canonical_with_namespace(
        slug,
        wing,
        collection_filter,
        None,
        include_superseded,
        false,
        conn,
    )
}

fn exact_slug_result_canonical_with_namespace(
    slug: &str,
    wing: Option<&str>,
    collection_filter: Option<i64>,
    namespace_filter: Option<&str>,
    include_superseded: bool,
    include_quarantined: bool,
    conn: &Connection,
) -> Result<Option<SearchResult>, SearchError> {
    if let Some(collection_id) = collection_filter {
        return exact_slug_result_canonical_for_collection_with_namespace(
            slug,
            wing,
            collection_id,
            namespace_filter,
            include_superseded,
            include_quarantined,
            conn,
        );
    }

    let resolved = match collections::parse_slug(conn, slug, OpKind::Read) {
        Ok(SlugResolution::Resolved {
            collection_id,
            collection_name,
            slug,
        }) => (collection_id, collection_name, slug),
        Ok(SlugResolution::NotFound { .. }) => {
            return Ok(None);
        }
        Ok(SlugResolution::Ambiguous { slug, candidates }) => {
            return Err(SearchError::Ambiguous {
                slug,
                candidates: candidates
                    .into_iter()
                    .map(|candidate| candidate.full_address)
                    .collect::<Vec<_>>()
                    .join(", "),
            });
        }
        Err(collections::CollectionError::Sqlite(err)) => return Err(SearchError::from(err)),
        Err(_) => return Ok(None),
    };

    let result = query_exact_slug_canonical(
        &resolved.2,
        wing,
        resolved.0,
        namespace_filter,
        include_superseded,
        include_quarantined,
        conn,
    );

    match result {
        Ok(result) => Ok(Some(result)),
        Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
        Err(err) => Err(SearchError::from(err)),
    }
}

#[cfg(test)]
fn exact_slug_result_canonical_for_collection(
    slug: &str,
    wing: Option<&str>,
    collection_id: i64,
    include_superseded: bool,
    conn: &Connection,
) -> Result<Option<SearchResult>, SearchError> {
    exact_slug_result_canonical_for_collection_with_namespace(
        slug,
        wing,
        collection_id,
        None,
        include_superseded,
        false,
        conn,
    )
}

fn exact_slug_result_canonical_for_collection_with_namespace(
    slug: &str,
    wing: Option<&str>,
    collection_id: i64,
    namespace_filter: Option<&str>,
    include_superseded: bool,
    include_quarantined: bool,
    conn: &Connection,
) -> Result<Option<SearchResult>, SearchError> {
    let stripped_slug = if let Some((collection_name, relative_slug)) = slug.split_once("::") {
        match collections::get_by_name(conn, collection_name) {
            Ok(Some(collection)) if collection.id == collection_id => relative_slug.to_owned(),
            Ok(Some(_)) | Ok(None) => return Ok(None),
            Err(collections::CollectionError::Sqlite(err)) => return Err(SearchError::from(err)),
            Err(_) => return Ok(None),
        }
    } else {
        slug.to_owned()
    };

    let result = query_exact_slug_canonical(
        &stripped_slug,
        wing,
        collection_id,
        namespace_filter,
        include_superseded,
        include_quarantined,
        conn,
    );

    match result {
        Ok(result) => Ok(Some(result)),
        Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
        Err(err) => Err(SearchError::from(err)),
    }
}

fn query_exact_slug_canonical(
    slug: &str,
    wing: Option<&str>,
    collection_id: i64,
    namespace_filter: Option<&str>,
    include_superseded: bool,
    include_quarantined: bool,
    conn: &Connection,
) -> Result<SearchResult, rusqlite::Error> {
    let mut query = String::from(
        "SELECT c.name || '::' || p.slug, p.title, p.summary, p.wing
         FROM pages p
         JOIN collections c ON c.id = p.collection_id
         WHERE p.collection_id = ?1 AND p.slug = ?2",
    );
    let mut params: Vec<Box<dyn rusqlite::types::ToSql>> =
        vec![Box::new(collection_id), Box::new(slug.to_owned())];

    if let Some(wing) = wing {
        query.push_str(" AND p.wing = ?");
        query.push_str(&(params.len() + 1).to_string());
        params.push(Box::new(wing.to_owned()));
    }
    if let Some(namespace) = namespace_filter {
        if namespace.is_empty() {
            query.push_str(" AND p.namespace = ?");
            query.push_str(&(params.len() + 1).to_string());
            params.push(Box::new(String::new()));
        } else {
            query.push_str(" AND (p.namespace = ?");
            query.push_str(&(params.len() + 1).to_string());
            query.push_str(" OR p.namespace = '')");
            params.push(Box::new(namespace.to_owned()));
        }
    }
    if !include_superseded {
        query.push_str(" AND p.superseded_by IS NULL");
    }
    if !include_quarantined {
        query.push_str(" AND p.quarantined_at IS NULL");
    }
    if let Some(namespace) = namespace_filter.filter(|namespace| !namespace.is_empty()) {
        query.push_str(" ORDER BY CASE WHEN p.namespace = ?");
        query.push_str(&(params.len() + 1).to_string());
        query.push_str(" THEN 0 ELSE 1 END");
        params.push(Box::new(namespace.to_owned()));
    }
    query.push_str(" LIMIT 1");

    let param_refs: Vec<&dyn rusqlite::types::ToSql> = params.iter().map(|p| p.as_ref()).collect();
    conn.query_row(&query, param_refs.as_slice(), |row| {
        Ok(SearchResult {
            slug: row.get(0)?,
            title: row.get(1)?,
            summary: row.get(2)?,
            score: 1.0,
            wing: row.get(3)?,
            ..Default::default()
        })
    })
}

fn merge_set_union(
    fts_results: &[SearchResult],
    vec_results: &[SearchResult],
) -> Vec<SearchResult> {
    let mut merged: HashMap<String, SearchResult> = HashMap::new();
    let fts_max = max_score(fts_results);
    let vec_max = max_score(vec_results);

    for result in fts_results {
        let normalized = normalize_score(result.score, fts_max);
        merged.insert(
            result.slug.clone(),
            SearchResult {
                score: normalized * 0.4,
                ..result.clone()
            },
        );
    }

    for result in vec_results {
        let normalized = normalize_score(result.score, vec_max) * 0.6;
        merged
            .entry(result.slug.clone())
            .and_modify(|existing| existing.score += normalized)
            .or_insert_with(|| SearchResult {
                score: normalized,
                ..result.clone()
            });
    }

    let mut results: Vec<_> = merged.into_values().collect();
    results.sort_by(|left, right| {
        right
            .score
            .total_cmp(&left.score)
            .then_with(|| left.slug.cmp(&right.slug))
    });
    results
}

fn merge_rrf(fts_results: &[SearchResult], vec_results: &[SearchResult]) -> Vec<SearchResult> {
    const RRF_K: f64 = 60.0;
    // Normalize the raw reciprocal-rank sum (max 2/(RRF_K+1) for a rank-0
    // hit in both lists) onto the same [0, 1] scale as set-union scores, so
    // one relevance floor and one gap-logging threshold work for both
    // strategies.
    let rrf_scale = (RRF_K + 1.0) / 2.0;

    let mut merged: HashMap<String, SearchResult> = HashMap::new();

    for (rank, result) in fts_results.iter().enumerate() {
        let contribution = 1.0 / (RRF_K + rank as f64 + 1.0);
        merged.insert(
            result.slug.clone(),
            SearchResult {
                score: contribution,
                ..result.clone()
            },
        );
    }

    for (rank, result) in vec_results.iter().enumerate() {
        let contribution = 1.0 / (RRF_K + rank as f64 + 1.0);
        merged
            .entry(result.slug.clone())
            .and_modify(|existing| existing.score += contribution)
            .or_insert_with(|| SearchResult {
                score: contribution,
                ..result.clone()
            });
    }

    for result in merged.values_mut() {
        result.score *= rrf_scale;
    }

    let mut results: Vec<_> = merged.into_values().collect();
    results.sort_by(|left, right| {
        right
            .score
            .total_cmp(&left.score)
            .then_with(|| left.slug.cmp(&right.slug))
    });
    results
}

fn max_score(results: &[SearchResult]) -> f64 {
    results
        .iter()
        .map(|result| result.score)
        .fold(0.0_f64, f64::max)
}

fn normalize_score(score: f64, max_score: f64) -> f64 {
    if max_score > 0.0 {
        score / max_score
    } else {
        0.0
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use rusqlite::{params, Connection};

    use super::*;
    use crate::commands::embed;
    use crate::core::db;
    use crate::core::fts::{sanitize_fts_query, search_fts, FtsQuery};
    use crate::core::inference::{search_vec, search_vec_canonical};

    fn open_test_db() -> Connection {
        let dir = tempfile::TempDir::new().expect("create temp dir");
        let db_path = dir.path().join("test_memory.db");
        let conn = db::open(db_path.to_str().expect("utf8 path")).expect("open db");
        std::mem::forget(dir);
        conn
    }

    fn result(slug: &str, score: f64) -> SearchResult {
        SearchResult {
            slug: slug.to_owned(),
            title: slug.to_owned(),
            summary: format!("summary for {slug}"),
            score,
            wing: "people".to_owned(),
            ..Default::default()
        }
    }

    fn insert_collection(conn: &Connection, name: &str) -> i64 {
        conn.execute(
            "INSERT INTO collections (name, root_path, state, writable, is_write_target)
             VALUES (?1, ?2, 'active', 1, 0)",
            params![name, format!("/{name}")],
        )
        .expect("insert collection");
        conn.last_insert_rowid()
    }

    fn insert_page(
        conn: &Connection,
        slug: &str,
        title: &str,
        summary: &str,
        truth: &str,
        wing: &str,
    ) {
        let mut hex = String::new();
        for byte in slug.as_bytes() {
            hex.push_str(&format!("{byte:02x}"));
            if hex.len() >= 32 {
                break;
            }
        }
        while hex.len() < 32 {
            hex.push('0');
        }
        let uuid = format!(
            "{}-{}-{}-{}-{}",
            &hex[0..8],
            &hex[8..12],
            &hex[12..16],
            &hex[16..20],
            &hex[20..32]
        );

        conn.execute(
            "INSERT INTO pages (slug, uuid, type, title, summary, compiled_truth, timeline, frontmatter, wing, room, version) \
             VALUES (?1, ?2, 'person', ?3, ?4, ?5, '', '{}', ?6, '', 1)",
            rusqlite::params![slug, uuid, title, summary, truth, wing],
        )
        .expect("insert page");
    }

    fn insert_page_in_collection(
        conn: &Connection,
        collection_id: i64,
        slug: &str,
        title: &str,
        summary: &str,
        truth: &str,
        wing: &str,
    ) {
        let mut hex = String::new();
        for byte in format!("{collection_id}-{slug}").as_bytes() {
            hex.push_str(&format!("{byte:02x}"));
            if hex.len() >= 32 {
                break;
            }
        }
        while hex.len() < 32 {
            hex.push('0');
        }
        let uuid = format!(
            "{}-{}-{}-{}-{}",
            &hex[0..8],
            &hex[8..12],
            &hex[12..16],
            &hex[16..20],
            &hex[20..32]
        );

        conn.execute(
            "INSERT INTO pages (collection_id, slug, uuid, type, title, summary, compiled_truth, timeline, frontmatter, wing, room, version) \
             VALUES (?1, ?2, ?3, 'person', ?4, ?5, ?6, '', '{}', ?7, '', 1)",
            rusqlite::params![collection_id, slug, uuid, title, summary, truth, wing],
        )
        .expect("insert page");
    }

    fn supersede_page(conn: &Connection, predecessor_slug: &str, successor_slug: &str) {
        let successor_id: i64 = conn
            .query_row(
                "SELECT id FROM pages WHERE slug = ?1",
                [successor_slug],
                |row| row.get(0),
            )
            .expect("successor id");
        conn.execute(
            "UPDATE pages SET superseded_by = ?1 WHERE slug = ?2",
            rusqlite::params![successor_id, predecessor_slug],
        )
        .expect("mark predecessor superseded");
    }

    #[test]
    fn hybrid_search_returns_empty_for_blank_query() {
        let conn = open_test_db();

        let results = hybrid_search(
            &conn,
            HybridSearch {
                query: "   ",
                limit: 1000,
                ..Default::default()
            },
        )
        .expect("hybrid search");

        assert!(results.is_empty());
    }

    #[test]
    fn hybrid_search_short_circuits_exact_slug_queries() {
        let conn = open_test_db();
        insert_page(
            &conn,
            "people/alice",
            "Alice",
            "Founder",
            "Alice works on AI agents.",
            "people",
        );

        let results = hybrid_search(
            &conn,
            HybridSearch {
                query: "people/alice",
                limit: 1000,
                ..Default::default()
            },
        )
        .expect("hybrid search");

        assert_eq!(results.len(), 1);
        assert_eq!(results[0].slug, "people/alice");
    }

    #[test]
    fn hybrid_search_short_circuits_wikilink_queries() {
        let conn = open_test_db();
        insert_page(
            &conn,
            "people/alice",
            "Alice",
            "Founder",
            "Alice works on AI agents.",
            "people",
        );

        let results = hybrid_search(
            &conn,
            HybridSearch {
                query: "[[people/alice]]",
                limit: 1000,
                ..Default::default()
            },
        )
        .expect("hybrid search");

        assert_eq!(results.len(), 1);
        assert_eq!(results[0].slug, "people/alice");
    }

    #[test]
    fn merge_set_union_returns_unique_results() {
        let fts = vec![result("a", 10.0), result("b", 9.0), result("c", 8.0)];
        let vec = vec![result("b", 8.0), result("c", 7.0), result("d", 6.0)];

        let results = merge_set_union(&fts, &vec);
        let slugs: Vec<_> = results.iter().map(|result| result.slug.as_str()).collect();

        assert_eq!(slugs.len(), 4);
        assert!(slugs.contains(&"a"));
        assert!(slugs.contains(&"b"));
        assert!(slugs.contains(&"c"));
        assert!(slugs.contains(&"d"));
    }

    #[test]
    fn hybrid_search_combines_fts_and_vector_results_without_exact_match() {
        let conn = open_test_db();
        insert_page(
            &conn,
            "people/alice",
            "Alice",
            "AI founder",
            "Alice works on AI agents and research.",
            "people",
        );
        insert_page(
            &conn,
            "people/bob",
            "Bob",
            "Cloud founder",
            "Cloud infrastructure and data systems.",
            "people",
        );
        insert_page(
            &conn,
            "companies/acme",
            "Acme",
            "AI company",
            "AI agents platform for founders.",
            "companies",
        );
        embed::run(&conn, None, true, false).expect("embed pages");

        let results = hybrid_search(
            &conn,
            HybridSearch {
                query: "AI founder",
                limit: 1000,
                ..Default::default()
            },
        )
        .expect("hybrid search");
        let slugs: Vec<_> = results.iter().map(|result| result.slug.as_str()).collect();

        assert!(slugs.contains(&"people/alice"));
        assert!(slugs.contains(&"companies/acme"));
    }

    #[test]
    fn hybrid_search_uses_tiered_fts_recall_when_and_and_vector_are_empty() {
        let conn = open_test_db();
        insert_page(
            &conn,
            "concepts/neural",
            "Neural Networks",
            "Representation learning",
            "Neural networks learn useful representations.",
            "concepts",
        );
        insert_page(
            &conn,
            "concepts/inference-engine",
            "Inference Engine",
            "Model serving",
            "An inference engine deploys trained models.",
            "concepts",
        );

        let sanitized = sanitize_fts_query("neural network inference");
        assert!(
            search_fts(
                &conn,
                FtsQuery {
                    query: &sanitized,
                    limit: 1000,
                    ..Default::default()
                }
            )
            .expect("AND-only FTS query")
            .is_empty(),
            "the deterministic proof corpus must force the implicit-AND FTS pass to miss"
        );
        assert!(
            search_vec("neural network inference", 10, None, None, &conn)
                .expect("vector search without embeddings")
                .is_empty(),
            "no embeddings are written in this proof, so vector recall must stay empty"
        );

        let results = hybrid_search(
            &conn,
            HybridSearch {
                query: "neural network inference",
                limit: 1000,
                ..Default::default()
            },
        )
        .expect("hybrid search");

        assert_eq!(
            results
                .iter()
                .map(|result| result.slug.clone())
                .collect::<BTreeSet<_>>(),
            BTreeSet::from([
                "concepts/inference-engine".to_owned(),
                "concepts/neural".to_owned(),
            ]),
            "hybrid_search should recover compound-term recall from the tiered FTS arm alone"
        );
    }

    #[test]
    fn hybrid_search_canonical_preserves_collection_prefix_on_tiered_fts_recall() {
        let conn = open_test_db();
        insert_page(
            &conn,
            "concepts/neural",
            "Neural Networks",
            "Representation learning",
            "Neural networks learn useful representations.",
            "concepts",
        );
        insert_page(
            &conn,
            "concepts/inference-engine",
            "Inference Engine",
            "Model serving",
            "An inference engine deploys trained models.",
            "concepts",
        );

        assert!(
            search_vec_canonical("neural network inference", 10, None, None, &conn)
                .expect("canonical vector search without embeddings")
                .is_empty(),
            "canonical hybrid proof must not depend on vector quality"
        );

        let results = hybrid_search(
            &conn,
            HybridSearch {
                query: "neural network inference",
                canonical: true,
                limit: 1000,
                ..Default::default()
            },
        )
        .expect("canonical hybrid search");

        assert_eq!(
            results
                .iter()
                .map(|result| result.slug.clone())
                .collect::<BTreeSet<_>>(),
            BTreeSet::from([
                "default::concepts/inference-engine".to_owned(),
                "default::concepts/neural".to_owned(),
            ]),
            "canonical hybrid results should keep the collection prefix on FTS fallback hits"
        );
    }

    #[test]
    fn exact_slug_result_canonical_hides_superseded_pages_by_default() {
        let conn = open_test_db();
        insert_page(
            &conn,
            "facts/a",
            "Fact A",
            "Archived",
            "Historical version",
            "facts",
        );
        insert_page(
            &conn,
            "facts/b",
            "Fact B",
            "Current",
            "Current version",
            "facts",
        );
        supersede_page(&conn, "facts/a", "facts/b");

        let default_result =
            exact_slug_result_canonical("facts/a", None, None, false, &conn).expect("head-only");
        let historical_result =
            exact_slug_result_canonical("facts/a", None, None, true, &conn).expect("history");

        assert!(default_result.is_none());
        assert_eq!(
            historical_result.expect("historical result").slug,
            "default::facts/a"
        );
    }

    #[test]
    fn hybrid_search_applies_wing_filter_to_both_subqueries() {
        let conn = open_test_db();
        insert_page(
            &conn,
            "people/alice",
            "Alice",
            "AI founder",
            "Alice works on AI agents and research.",
            "people",
        );
        insert_page(
            &conn,
            "companies/acme",
            "Acme",
            "AI company",
            "AI agents platform for founders.",
            "companies",
        );
        embed::run(&conn, None, true, false).expect("embed pages");

        let results = hybrid_search(
            &conn,
            HybridSearch {
                query: "AI founder",
                wing: Some("people"),
                limit: 1000,
                ..Default::default()
            },
        )
        .expect("hybrid search");

        assert!(!results.is_empty());
        assert!(results.iter().all(|result| result.wing == "people"));
    }

    #[test]
    fn hybrid_search_namespace_filter_includes_global_and_excludes_other_namespaces() {
        let conn = open_test_db();
        insert_page(
            &conn,
            "notes/global",
            "Global",
            "Global",
            "sharedtoken",
            "notes",
        );
        conn.execute(
            "INSERT INTO pages
                 (namespace, slug, uuid, type, title, summary, compiled_truth, timeline, frontmatter, wing, room, version)
             VALUES ('test-ns', 'notes/private', ?1, 'concept', 'Private', 'Private', 'privatetoken', '', '{}', 'notes', '', 1)",
            [uuid::Uuid::now_v7().to_string()],
        )
        .expect("insert namespaced page");

        let namespaced = hybrid_search(
            &conn,
            HybridSearch {
                query: "sharedtoken privatetoken",
                namespace: Some("test-ns"),
                canonical: true,
                limit: 10,
                ..Default::default()
            },
        )
        .expect("namespaced query");
        let global_only = hybrid_search(
            &conn,
            HybridSearch {
                query: "privatetoken",
                namespace: Some(""),
                canonical: true,
                limit: 10,
                ..Default::default()
            },
        )
        .expect("global query");

        assert!(namespaced
            .iter()
            .any(|result| result.slug == "default::notes/global"));
        assert!(namespaced
            .iter()
            .any(|result| result.slug == "default::notes/private"));
        assert!(global_only.is_empty());
    }

    #[test]
    fn read_merge_strategy_defaults_to_set_union() {
        let conn = open_test_db();

        let strategy = read_merge_strategy(&conn).expect("merge strategy");

        assert_eq!(strategy, SearchMergeStrategy::SetUnion);
    }

    #[test]
    fn read_merge_strategy_honors_rrf_config() {
        let conn = open_test_db();
        conn.execute(
            "INSERT INTO config (key, value) VALUES ('search_merge_strategy', 'rrf')
             ON CONFLICT(key) DO UPDATE SET value = excluded.value",
            [],
        )
        .expect("insert config");

        let strategy = read_merge_strategy(&conn).expect("merge strategy");

        assert_eq!(strategy, SearchMergeStrategy::Rrf);
    }

    #[test]
    fn read_merge_strategy_propagates_sql_errors() {
        let conn = open_test_db();
        conn.execute("DROP TABLE config", []).expect("drop config");

        let err = read_merge_strategy(&conn).expect_err("missing config table should fail");

        assert!(matches!(err, SearchError::Sqlite(_)));
    }

    #[test]
    fn hybrid_search_uses_rrf_when_configured() {
        let conn = open_test_db();
        insert_page(
            &conn,
            "concepts/rust",
            "Rust",
            "Systems language",
            "Rust is a systems programming language focused on memory safety.",
            "concepts",
        );
        conn.execute(
            "INSERT INTO config (key, value) VALUES ('search_merge_strategy', 'rrf')
             ON CONFLICT(key) DO UPDATE SET value = excluded.value",
            [],
        )
        .expect("insert config");

        let results = hybrid_search(
            &conn,
            HybridSearch {
                query: "systems language",
                limit: 1,
                ..Default::default()
            },
        )
        .expect("rrf search");

        assert_eq!(results.len(), 1);
        assert_eq!(results[0].slug, "concepts/rust");
    }

    #[test]
    fn exact_slug_result_applies_wing_and_collection_filters() {
        let conn = open_test_db();
        let default_collection_id: i64 = conn
            .query_row(
                "SELECT id FROM collections WHERE name = 'default'",
                [],
                |row| row.get(0),
            )
            .expect("default collection id");
        insert_page(
            &conn,
            "people/alice",
            "Alice",
            "Founder",
            "Alice works on AI agents.",
            "people",
        );

        let matching = exact_slug_result(
            "people/alice",
            Some("people"),
            Some(default_collection_id),
            false,
            &conn,
            false,
        )
        .expect("exact slug");
        let wrong_wing =
            exact_slug_result("people/alice", Some("companies"), None, false, &conn, false)
                .expect("wrong wing");
        let wrong_collection = exact_slug_result(
            "people/alice",
            None,
            Some(default_collection_id + 999),
            false,
            &conn,
            false,
        )
        .expect("wrong collection");

        assert_eq!(matching.expect("matching result").slug, "people/alice");
        assert!(wrong_wing.is_none());
        assert!(wrong_collection.is_none());
    }

    #[test]
    fn exact_slug_result_propagates_sql_errors() {
        let conn = open_test_db();
        conn.execute("DROP TABLE pages", []).expect("drop pages");

        let err = exact_slug_result("people/alice", None, None, false, &conn, false)
            .expect_err("missing pages table should fail");

        assert!(matches!(err, SearchError::Sqlite(_)));
    }

    #[test]
    fn hybrid_search_canonical_returns_ambiguous_error_for_exact_slug_query() {
        let conn = open_test_db();
        let work_collection_id = insert_collection(&conn, "work");
        insert_page(
            &conn,
            "people/alice",
            "Alice Default",
            "Founder",
            "Alice works on default agents.",
            "people",
        );
        insert_page_in_collection(
            &conn,
            work_collection_id,
            "people/alice",
            "Alice Work",
            "Operator",
            "Alice works on the work collection agents.",
            "people",
        );

        let err = hybrid_search(
            &conn,
            HybridSearch {
                query: "people/alice",
                canonical: true,
                limit: 10,
                ..Default::default()
            },
        )
        .expect_err("ambiguous canonical search should fail");

        assert!(matches!(
            err,
            SearchError::Ambiguous { slug, candidates }
                if slug == "people/alice"
                    && candidates.contains("default::people/alice")
                    && candidates.contains("work::people/alice")
        ));
    }

    #[test]
    fn exact_slug_result_canonical_returns_prefixed_slug_for_explicit_collection_address() {
        let conn = open_test_db();
        let work_collection_id = insert_collection(&conn, "work");
        insert_page_in_collection(
            &conn,
            work_collection_id,
            "people/alice",
            "Alice Work",
            "Operator",
            "Alice works on the work collection agents.",
            "people",
        );

        let result =
            exact_slug_result_canonical("work::people/alice", Some("people"), None, false, &conn)
                .expect("canonical exact slug")
                .expect("matching page");

        assert_eq!(result.slug, "work::people/alice");
        assert_eq!(result.wing, "people");
    }

    #[test]
    fn exact_slug_result_canonical_returns_none_when_wing_filter_excludes_match() {
        let conn = open_test_db();
        let work_collection_id = insert_collection(&conn, "work");
        insert_page_in_collection(
            &conn,
            work_collection_id,
            "people/alice",
            "Alice Work",
            "Operator",
            "Alice works on the work collection agents.",
            "people",
        );

        let result = exact_slug_result_canonical(
            "work::people/alice",
            Some("companies"),
            None,
            false,
            &conn,
        )
        .expect("canonical exact slug");

        assert!(result.is_none());
    }

    #[test]
    fn exact_slug_result_canonical_for_collection_propagates_collection_lookup_sql_errors() {
        let conn = open_test_db();
        conn.execute("DROP TABLE collections", [])
            .expect("drop collections");

        let err =
            exact_slug_result_canonical_for_collection("work::people/alice", None, 1, false, &conn)
                .expect_err("missing collections table should fail");

        assert!(matches!(err, SearchError::Sqlite(_)));
    }

    #[test]
    fn exact_slug_result_canonical_returns_none_for_invalid_bare_slug() {
        let conn = open_test_db();

        let result = exact_slug_result_canonical("/etc/passwd", None, None, false, &conn)
            .expect("invalid slug");

        assert!(result.is_none());
    }

    #[test]
    fn exact_slug_result_canonical_for_collection_handles_prefix_and_wing_filters() {
        let conn = open_test_db();
        let work_collection_id = insert_collection(&conn, "work");
        insert_page_in_collection(
            &conn,
            work_collection_id,
            "people/alice",
            "Alice Work",
            "Operator",
            "Alice works on the work collection agents.",
            "people",
        );

        let matching = exact_slug_result_canonical_for_collection(
            "work::people/alice",
            Some("people"),
            work_collection_id,
            false,
            &conn,
        )
        .expect("matching canonical exact slug");
        let wrong_prefix = exact_slug_result_canonical_for_collection(
            "default::people/alice",
            None,
            work_collection_id,
            false,
            &conn,
        )
        .expect("wrong prefix");
        let missing_prefix = exact_slug_result_canonical_for_collection(
            "missing::people/alice",
            None,
            work_collection_id,
            false,
            &conn,
        )
        .expect("missing prefix");
        let wrong_wing = exact_slug_result_canonical_for_collection(
            "people/alice",
            Some("companies"),
            work_collection_id,
            false,
            &conn,
        )
        .expect("wrong wing");

        assert_eq!(
            matching.expect("matching result").slug,
            "work::people/alice"
        );
        assert!(wrong_prefix.is_none());
        assert!(missing_prefix.is_none());
        assert!(wrong_wing.is_none());
    }

    #[test]
    fn merge_rrf_combines_ranked_results() {
        let fts = vec![result("a", 10.0), result("b", 9.0)];
        let vec = vec![result("b", 8.0), result("c", 7.0)];

        let results = merge_rrf(&fts, &vec);
        let slugs: Vec<_> = results.iter().map(|result| result.slug.as_str()).collect();

        assert_eq!(slugs, vec!["b", "a", "c"]);
        assert!(results[0].score > results[1].score);
        // Normalized scale: a dual-list hit at ranks 1 and 0 lands just
        // below 1.0; single-list hits land near 0.5.
        assert!(
            results[0].score > 0.9 && results[0].score <= 1.0,
            "dual-list RRF hit must score near 1.0, got {}",
            results[0].score
        );
        assert!(
            (results[1].score - 0.5).abs() < 0.01,
            "rank-0 single-list RRF hit must score ~0.5, got {}",
            results[1].score
        );
    }

    #[test]
    fn merge_rrf_dual_rank_zero_hit_scores_exactly_one() {
        let fts = vec![result("a", 10.0)];
        let vec = vec![result("a", 8.0)];

        let results = merge_rrf(&fts, &vec);

        assert_eq!(results.len(), 1);
        assert!(
            (results[0].score - 1.0).abs() < 1e-12,
            "rank-0 hit in both lists must normalize to 1.0, got {}",
            results[0].score
        );
    }

    #[test]
    fn normalize_score_returns_zero_when_max_is_zero() {
        assert_eq!(normalize_score(5.0, 0.0), 0.0);
        assert_eq!(max_score(&[]), 0.0);
    }

    /// Regression: issue #37 — question marks in natural-language queries must
    /// not trigger FTS5 syntax errors in the hybrid search path.
    #[test]
    fn hybrid_search_accepts_question_mark_in_query() {
        let conn = open_test_db();
        insert_page(
            &conn,
            "concepts/rust",
            "Rust",
            "Systems language",
            "Rust is a systems programming language focused on safety.",
            "concepts",
        );
        embed::run(&conn, None, true, false).expect("embed pages");

        let results = hybrid_search(
            &conn,
            HybridSearch {
                query: "what is rust?",
                limit: 1000,
                ..Default::default()
            },
        )
        .expect("hybrid search with ?");
        assert!(!results.is_empty());
    }

    /// Regression: issue #37 — "AND?" must be safe on the natural-language path
    /// but still yield no results because no content-bearing terms survive.
    #[test]
    fn hybrid_search_returns_empty_for_operator_only_query() {
        let conn = open_test_db();
        insert_page(
            &conn,
            "concepts/rust",
            "Rust",
            "Systems language",
            "Rust is a systems programming language focused on safety.",
            "concepts",
        );
        embed::run(&conn, None, true, false).expect("embed pages");

        let results = hybrid_search(
            &conn,
            HybridSearch {
                query: "AND?",
                limit: 1000,
                ..Default::default()
            },
        )
        .expect("hybrid search with AND?");
        assert!(results.is_empty());
    }

    #[test]
    fn hybrid_search_returns_empty_for_punctuation_only_query() {
        let conn = open_test_db();
        insert_page(
            &conn,
            "concepts/rust",
            "Rust",
            "Systems language",
            "Rust is a systems programming language focused on safety.",
            "concepts",
        );
        embed::run(&conn, None, true, false).expect("embed pages");

        let results = hybrid_search(
            &conn,
            HybridSearch {
                query: "???***",
                limit: 1000,
                ..Default::default()
            },
        )
        .expect("hybrid search with punctuation only");
        assert!(results.is_empty());
    }

    /// Regression: review blocker — commas, periods, apostrophes, slashes,
    /// semicolons, and `=` all trigger FTS5 syntax errors when passed raw.
    /// hybrid_search must sanitize all of them on the natural-language path.
    #[test]
    fn hybrid_search_accepts_comma_and_period_in_query() {
        let conn = open_test_db();
        insert_page(
            &conn,
            "concepts/rust",
            "Rust",
            "Systems language",
            "Rust is a systems programming language focused on safety.",
            "concepts",
        );
        embed::run(&conn, None, true, false).expect("embed pages");

        assert!(hybrid_search(
            &conn,
            HybridSearch {
                query: "hello, world.",
                limit: 1000,
                ..Default::default()
            },
        )
        .is_ok());
    }

    #[test]
    fn hybrid_search_accepts_apostrophe_in_query() {
        let conn = open_test_db();
        insert_page(
            &conn,
            "concepts/rust",
            "Rust",
            "Systems language",
            "Rust is a systems programming language.",
            "concepts",
        );
        embed::run(&conn, None, true, false).expect("embed pages");

        assert!(hybrid_search(
            &conn,
            HybridSearch {
                query: "what's rust's type system?",
                limit: 1000,
                ..Default::default()
            },
        )
        .is_ok());
    }

    #[test]
    fn hybrid_search_accepts_slash_and_equals_in_query() {
        let conn = open_test_db();
        insert_page(
            &conn,
            "concepts/rust",
            "Rust",
            "Systems language",
            "Rust is a systems programming language.",
            "concepts",
        );
        embed::run(&conn, None, true, false).expect("embed pages");

        assert!(hybrid_search(
            &conn,
            HybridSearch {
                query: "path/to/thing key=value",
                limit: 1000,
                ..Default::default()
            },
        )
        .is_ok());
    }

    #[test]
    fn hybrid_search_accepts_semicolon_in_query() {
        let conn = open_test_db();
        insert_page(
            &conn,
            "concepts/rust",
            "Rust",
            "Systems language",
            "Rust is a systems programming language.",
            "concepts",
        );
        embed::run(&conn, None, true, false).expect("embed pages");

        assert!(hybrid_search(
            &conn,
            HybridSearch {
                query: "memory; safety",
                limit: 1000,
                ..Default::default()
            },
        )
        .is_ok());
    }
}
