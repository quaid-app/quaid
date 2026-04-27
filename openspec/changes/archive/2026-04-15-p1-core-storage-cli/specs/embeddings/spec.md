## ADDED Requirements

### Requirement: Candle model initialization
The system SHALL initialize BGE-small-en-v1.5 via candle at most once per process using
a `OnceLock<EmbeddingModel>` in `src/core/inference.rs`. The model weights SHALL be
embedded in the binary via `include_bytes!` by default. The `online-model` feature flag
SHALL skip embedding weights and download them to `~/.quaid/models/` on first call.

#### Scenario: Model initialised on first inference call
- **WHEN** `embed("hello world")` is called for the first time in a process
- **THEN** the model is initialised (weights loaded, tokenizer ready) and a 384-dimensional embedding is returned

#### Scenario: Model not re-initialised on subsequent calls
- **WHEN** `embed("second call")` is called after the model is already initialized
- **THEN** the model is reused from `OnceLock` without re-loading weights

#### Scenario: CPU device used by default
- **WHEN** `embed(text)` is called without any GPU feature enabled
- **THEN** the computation runs on CPU and returns a valid embedding vector

### Requirement: Text embedding generation
The `embed(text: &str)` function SHALL return a 384-dimensional `Vec<f32>` embedding
for any input text using the BGE-small-en-v1.5 model. Embeddings SHALL be L2-normalized.

#### Scenario: Single text embedding
- **WHEN** `embed("Alice works at Acme Corp")` is called
- **THEN** a `Vec<f32>` of length 384 is returned with L2 norm ≈ 1.0

#### Scenario: Empty string embedding
- **WHEN** `embed("")` is called
- **THEN** the function returns `Err(InferenceError::EmptyInput)` without crashing

### Requirement: Temporal sub-chunking
The chunking module SHALL split a page's `compiled_truth` into section-level chunks
at `##` boundaries, and split the `timeline` into individual timeline entries. Each
chunk SHALL be stored with its `content_hash` (SHA-256), `token_count`, and `heading_path`.

#### Scenario: Compiled_truth split at ## headers
- **WHEN** `chunk_page(page)` is called on a page with three `##` sections
- **THEN** three compiled_truth chunks are produced, each with its heading path

#### Scenario: Timeline entries split individually
- **WHEN** `chunk_page(page)` is called on a page with five timeline entries (separated by `---`)
- **THEN** five timeline chunks are produced

#### Scenario: Chunks have content_hash
- **WHEN** any chunk is produced by `chunk_page`
- **THEN** the chunk has a `content_hash` field equal to SHA-256 of the chunk text

### Requirement: Vector search
`search_vec(query: &str, k: usize, wing_filter: Option<&str>, conn: &Connection)`
SHALL embed the query, query the `page_embeddings_vec_384` vec0 table for the `k`
nearest neighbours by cosine similarity, and return ranked `SearchResult` items.

#### Scenario: Vector search returns top-k results
- **WHEN** `search_vec("board member tech company", 5, None, &conn)` is called
- **THEN** up to 5 pages are returned ranked by cosine similarity

#### Scenario: Wing-filtered vector search
- **WHEN** `search_vec("startup founder", 10, Some("people"), &conn)` is called
- **THEN** only pages with `wing = 'people'` are in the result set

#### Scenario: No embeddings in database
- **WHEN** `search_vec("any query", 5, None, &conn)` is called on a database with no embeddings
- **THEN** an empty result set is returned without error

### Requirement: quaid embed command
`quaid embed [SLUG]` SHALL generate and store embeddings for a single page.
`quaid embed --all` SHALL generate embeddings for all pages.
`quaid embed --stale` SHALL regenerate embeddings for pages whose `content_hash` has
changed since last embedding.

#### Scenario: Embed single page
- **WHEN** `quaid embed people/alice` is called
- **THEN** all chunks for `people/alice` are embedded and stored in `page_embeddings`

#### Scenario: Embed all pages
- **WHEN** `quaid embed --all` is called
- **THEN** all pages in the database are embedded (skipping already-embedded unchanged chunks)

### Requirement: quaid query command
`quaid query "<QUERY>" [--depth auto|1|2] [--token-budget N] [--wing WING]`
SHALL perform hybrid search and print the top results with their summaries.
The `--depth` flag controls progressive expansion (Phase 2 feature; in Phase 1, depth
always returns full page content for matched results). `--token-budget` caps output.

#### Scenario: Basic semantic query
- **WHEN** `quaid query "who is working on AI agents?"` is called
- **THEN** hybrid search is performed and the top results are printed with slug + summary
