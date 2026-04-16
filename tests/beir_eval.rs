//! BEIR retrieval regression gate — offline CI benchmark.
//!
//! Evaluates hybrid search quality on NQ and FiQA subsets using nDCG@10.
//! Compares results against the pinned baseline in `benchmarks/baselines/beir.json`.
//! Fails if regression exceeds 2%.
//!
//! All tests are `#[ignore]` — they require datasets downloaded by:
//!     ./benchmarks/prep_datasets.sh
//!
//! Run with:
//!     cargo test --test beir_eval -- --ignored
//!     cargo test --test beir_eval fiqa -- --ignored    # FiQA only
//!
//! CI integration: see task 7.2 (BEIR regression job on release branches).

use std::collections::HashMap;
use std::fs;
use std::path::Path;

use gbrain::commands::embed;
use gbrain::core::db;
use gbrain::core::migrate::import_dir;
use gbrain::core::search::hybrid_search;

// ── Dataset paths ─────────────────────────────────────────────────────────────

fn datasets_dir() -> std::path::PathBuf {
    let manifest = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    manifest.join("benchmarks").join("datasets")
}

fn baselines_path() -> std::path::PathBuf {
    let manifest = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    manifest
        .join("benchmarks")
        .join("baselines")
        .join("beir.json")
}

// ── BEIR dataset types ────────────────────────────────────────────────────────

#[derive(Debug, serde::Deserialize)]
struct BeirDoc {
    #[serde(rename = "_id")]
    id: String,
    title: String,
    text: String,
}

#[derive(Debug, serde::Deserialize)]
struct BeirQuery {
    #[serde(rename = "_id")]
    id: String,
    text: String,
}

/// Parsed relevance judgments from qrels TSV.
/// Maps query_id → {doc_id → relevance_score}.
type Qrels = HashMap<String, HashMap<String, u32>>;

// ── BEIR loader ───────────────────────────────────────────────────────────────

fn load_corpus_jsonl(path: &Path) -> Vec<BeirDoc> {
    let content = fs::read_to_string(path)
        .unwrap_or_else(|e| panic!("Failed to read corpus: {e}\nPath: {}", path.display()));
    content
        .lines()
        .filter(|l| !l.trim().is_empty())
        .map(|line| serde_json::from_str(line).expect("parse corpus JSONL"))
        .collect()
}

fn load_queries_jsonl(path: &Path) -> Vec<BeirQuery> {
    let content = fs::read_to_string(path)
        .unwrap_or_else(|e| panic!("Failed to read queries: {e}\nPath: {}", path.display()));
    content
        .lines()
        .filter(|l| !l.trim().is_empty())
        .map(|line| serde_json::from_str(line).expect("parse queries JSONL"))
        .collect()
}

fn load_qrels_tsv(path: &Path) -> Qrels {
    let content = fs::read_to_string(path)
        .unwrap_or_else(|e| panic!("Failed to read qrels: {e}\nPath: {}", path.display()));
    let mut qrels: Qrels = HashMap::new();
    for line in content.lines().skip(1) {
        // Skip header
        let cols: Vec<&str> = line.split('\t').collect();
        if cols.len() < 3 {
            continue;
        }
        let (qid, did, score): (&str, &str, u32) =
            (cols[0], cols[1], cols[2].trim().parse().unwrap_or(0));
        if score > 0 {
            qrels
                .entry(qid.to_string())
                .or_default()
                .insert(did.to_string(), score);
        }
    }
    qrels
}

// ── nDCG@10 computation ───────────────────────────────────────────────────────

/// Compute Discounted Cumulative Gain for retrieved docs against qrels.
fn dcg(retrieved: &[String], relevant: &HashMap<String, u32>, k: usize) -> f64 {
    retrieved
        .iter()
        .take(k)
        .enumerate()
        .map(|(rank, doc_id)| {
            let rel = *relevant.get(doc_id).unwrap_or(&0) as f64;
            rel / (rank as f64 + 2.0).log2()
        })
        .sum()
}

/// Ideal DCG: best-case ordering of relevant docs.
fn idcg(relevant: &HashMap<String, u32>, k: usize) -> f64 {
    let mut scores: Vec<f64> = relevant.values().map(|&s| s as f64).collect();
    scores.sort_by(|a, b| b.total_cmp(a));
    scores
        .iter()
        .take(k)
        .enumerate()
        .map(|(rank, &rel)| rel / (rank as f64 + 2.0).log2())
        .sum()
}

/// nDCG@k for a single query.
fn ndcg_at_k(retrieved: &[String], relevant: &HashMap<String, u32>, k: usize) -> f64 {
    let ideal = idcg(relevant, k);
    if ideal == 0.0 {
        return 1.0; // No relevant docs — vacuously perfect
    }
    dcg(retrieved, relevant, k) / ideal
}

/// Mean nDCG@10 across all evaluated queries.
fn mean_ndcg_at_10(results: &[(Vec<String>, HashMap<String, u32>)]) -> f64 {
    if results.is_empty() {
        return 0.0;
    }
    let sum: f64 = results
        .iter()
        .map(|(retrieved, relevant)| ndcg_at_k(retrieved, relevant, 10))
        .sum();
    sum / results.len() as f64
}

// ── Importer: BEIR corpus → gbrain pages ─────────────────────────────────────

/// Write BEIR documents as gbrain markdown pages to a temp directory,
/// then bulk-import with `import_dir`.
fn import_beir_corpus(
    conn: &rusqlite::Connection,
    docs: &[BeirDoc],
    wing: &str,
) -> anyhow::Result<usize> {
    let dir = tempfile::TempDir::new()?;
    let corpus_path = dir.path().join(wing);
    fs::create_dir_all(&corpus_path)?;

    for doc in docs.iter().take(10_000) {
        // Cap at 10k docs for tractable test runs
        let slug = sanitize_slug(&doc.id);
        let content = format!(
            "---\nslug: {wing}/{slug}\ntitle: {}\ntype: document\nwing: {wing}\n---\n{}\n\n{}\n",
            escape_yaml(&doc.title),
            doc.title,
            doc.text
        );
        let path = corpus_path.join(format!("{slug}.md"));
        fs::write(&path, &content)?;
    }

    let stats = import_dir(conn, dir.path(), false)?;
    std::mem::forget(dir); // keep files alive during import
    Ok(stats.imported)
}

fn sanitize_slug(id: &str) -> String {
    id.chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '-'
            }
        })
        .collect::<String>()
        .trim_matches('-')
        .to_string()
}

fn escape_yaml(s: &str) -> String {
    if s.contains(':') || s.contains('#') || s.contains('\'') {
        format!("\"{}\"", s.replace('"', "\\\""))
    } else {
        s.to_string()
    }
}

// ── Baseline loader ───────────────────────────────────────────────────────────

fn load_baseline() -> serde_json::Value {
    let path = baselines_path();
    let content = fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("baseline not found at {}: {e}", path.display()));
    serde_json::from_str(&content).expect("parse baselines/beir.json")
}

fn baseline_ndcg(dataset: &str) -> Option<f64> {
    let baseline = load_baseline();
    baseline["baselines"][dataset]["ndcg_at_10"].as_f64()
}

fn regression_threshold() -> f64 {
    let baseline = load_baseline();
    baseline["regression_threshold_pct"].as_f64().unwrap_or(2.0)
}

// ── FiQA evaluation ───────────────────────────────────────────────────────────

#[test]
#[ignore = "requires FiQA dataset — run: ./benchmarks/prep_datasets.sh fiqa"]
fn beir_fiqa_ndcg_at_10_meets_baseline() {
    let fiqa_dir = datasets_dir().join("beir").join("fiqa").join("fiqa");
    assert!(
        fiqa_dir.exists(),
        "FiQA dataset not found at {}. Run: ./benchmarks/prep_datasets.sh fiqa",
        fiqa_dir.display()
    );

    let corpus = load_corpus_jsonl(&fiqa_dir.join("corpus.jsonl"));
    let queries = load_queries_jsonl(&fiqa_dir.join("queries.jsonl"));
    let qrels = load_qrels_tsv(&fiqa_dir.join("qrels").join("test.tsv"));

    eprintln!(
        "FiQA: {} corpus docs, {} queries, {} evaluated queries",
        corpus.len(),
        queries.len(),
        qrels.len()
    );

    let db_dir = tempfile::TempDir::new().unwrap();
    let db_path = db_dir.path().join("bench.db");
    let conn = db::open(db_path.to_str().unwrap()).unwrap();

    let imported = import_beir_corpus(&conn, &corpus, "fiqa").unwrap();
    eprintln!("Imported {} FiQA documents", imported);

    embed::run(&conn, None, true, false).expect("embed pages");

    let test_queries: Vec<&BeirQuery> = queries
        .iter()
        .filter(|q| qrels.contains_key(&q.id))
        .take(500)
        .collect();

    let mut eval_results: Vec<(Vec<String>, HashMap<String, u32>)> = Vec::new();

    for query in &test_queries {
        let results = hybrid_search(&query.text, None, &conn, 10).unwrap_or_default();
        let retrieved: Vec<String> = results
            .iter()
            .map(|r| r.slug.strip_prefix("fiqa/").unwrap_or(&r.slug).to_string())
            .collect();
        let relevant = qrels.get(&query.id).cloned().unwrap_or_default();
        eval_results.push((retrieved, relevant));
    }

    let ndcg = mean_ndcg_at_10(&eval_results);
    eprintln!(
        "FiQA nDCG@10: {:.4} (over {} queries)",
        ndcg,
        test_queries.len()
    );

    // Update baseline if not yet set
    let Some(baseline) = baseline_ndcg("fiqa") else {
        eprintln!(
            "⚠  No FiQA baseline established yet. Current score: {:.4}. \
             Update benchmarks/baselines/beir.json to record this as the anchor.",
            ndcg
        );
        return;
    };

    let threshold = regression_threshold();
    let min_acceptable = baseline * (1.0 - threshold / 100.0);

    assert!(
        ndcg >= min_acceptable,
        "FiQA nDCG@10 regression detected!\n  baseline:  {:.4}\n  current:   {:.4}\n  threshold: {:.1}%\n  min:       {:.4}",
        baseline,
        ndcg,
        threshold,
        min_acceptable
    );

    eprintln!(
        "✓ FiQA nDCG@10 {:.4} ≥ threshold {:.4}",
        ndcg, min_acceptable
    );
}

// ── NQ evaluation ─────────────────────────────────────────────────────────────

#[test]
#[ignore = "requires NQ dataset (~490 MB) — run: ./benchmarks/prep_datasets.sh nq"]
fn beir_nq_ndcg_at_10_meets_baseline() {
    let nq_dir = datasets_dir().join("beir").join("nq").join("nq");
    assert!(
        nq_dir.exists(),
        "NQ dataset not found at {}. Run: ./benchmarks/prep_datasets.sh nq",
        nq_dir.display()
    );

    let corpus = load_corpus_jsonl(&nq_dir.join("corpus.jsonl"));
    let queries = load_queries_jsonl(&nq_dir.join("queries.jsonl"));
    let qrels = load_qrels_tsv(&nq_dir.join("qrels").join("test.tsv"));

    eprintln!(
        "NQ: {} corpus docs, {} queries, {} evaluated queries",
        corpus.len(),
        queries.len(),
        qrels.len()
    );

    let db_dir = tempfile::TempDir::new().unwrap();
    let db_path = db_dir.path().join("bench.db");
    let conn = db::open(db_path.to_str().unwrap()).unwrap();

    // NQ corpus is 2.6M docs — cap at 50k for tractable local runs
    let imported = import_beir_corpus(&conn, &corpus, "nq").unwrap();
    eprintln!("Imported {} NQ documents (capped at 10k)", imported);

    embed::run(&conn, None, true, false).expect("embed pages");

    let test_queries: Vec<&BeirQuery> = queries
        .iter()
        .filter(|q| qrels.contains_key(&q.id))
        .take(200)
        .collect();

    let mut eval_results: Vec<(Vec<String>, HashMap<String, u32>)> = Vec::new();

    for query in &test_queries {
        let results = hybrid_search(&query.text, None, &conn, 10).unwrap_or_default();
        let retrieved: Vec<String> = results
            .iter()
            .map(|r| r.slug.strip_prefix("nq/").unwrap_or(&r.slug).to_string())
            .collect();
        let relevant = qrels.get(&query.id).cloned().unwrap_or_default();
        eval_results.push((retrieved, relevant));
    }

    let ndcg = mean_ndcg_at_10(&eval_results);
    eprintln!(
        "NQ nDCG@10: {:.4} (over {} queries)",
        ndcg,
        test_queries.len()
    );

    let Some(baseline) = baseline_ndcg("nq") else {
        eprintln!(
            "⚠  No NQ baseline established yet. Current score: {:.4}. \
             Update benchmarks/baselines/beir.json to record this as the anchor.",
            ndcg
        );
        return;
    };

    let threshold = regression_threshold();
    let min_acceptable = baseline * (1.0 - threshold / 100.0);

    assert!(
        ndcg >= min_acceptable,
        "NQ nDCG@10 regression detected!\n  baseline:  {:.4}\n  current:   {:.4}\n  threshold: {:.1}%\n  min:       {:.4}",
        baseline,
        ndcg,
        threshold,
        min_acceptable
    );

    eprintln!("✓ NQ nDCG@10 {:.4} ≥ threshold {:.4}", ndcg, min_acceptable);
}

// ── Regression detection unit test (no datasets required) ────────────────────

#[test]
fn ndcg_computation_is_correct() {
    // Single query: 2 relevant docs at ranks 1 and 3
    let retrieved = vec![
        "doc1".to_string(),
        "doc_irrel".to_string(),
        "doc2".to_string(),
    ];
    let mut relevant = HashMap::new();
    relevant.insert("doc1".to_string(), 1u32);
    relevant.insert("doc2".to_string(), 1u32);

    let dcg_actual = dcg(&retrieved, &relevant, 10);
    // DCG = 1/log2(2) + 0 + 1/log2(4) = 1.0 + 0.5 = 1.5
    assert!((dcg_actual - 1.5).abs() < 1e-6, "DCG={dcg_actual}");

    let ideal = idcg(&relevant, 10);
    // Ideal: doc1 at rank 1, doc2 at rank 2: 1/log2(2) + 1/log2(3) ≈ 1.0 + 0.631 = 1.631
    let expected_ideal = 1.0 + 1.0_f64 / 3.0_f64.log2();
    assert!(
        (ideal - expected_ideal).abs() < 1e-6,
        "iDCG={ideal}, expected={expected_ideal}"
    );

    let ndcg = ndcg_at_k(&retrieved, &relevant, 10);
    assert!(
        ndcg > 0.9,
        "nDCG@10 should be high for 2/2 relevant in top-3: {ndcg}"
    );
    assert!(
        ndcg < 1.0,
        "nDCG should be < 1 (doc2 not at rank 2): {ndcg}"
    );
}

#[test]
fn ndcg_is_perfect_for_ideal_ranking() {
    let retrieved = vec!["doc1".to_string(), "doc2".to_string(), "doc3".to_string()];
    let mut relevant = HashMap::new();
    relevant.insert("doc1".to_string(), 1u32);
    relevant.insert("doc2".to_string(), 1u32);
    relevant.insert("doc3".to_string(), 1u32);

    let ndcg = ndcg_at_k(&retrieved, &relevant, 10);
    assert!(
        (ndcg - 1.0).abs() < 1e-6,
        "ideal ranking should give nDCG=1: {ndcg}"
    );
}

#[test]
fn regression_detection_fires_at_threshold() {
    // 2% drop should NOT be flagged (exactly at threshold boundary)
    let baseline = 0.500;
    let threshold = 2.0_f64;
    let min_acceptable = baseline * (1.0 - threshold / 100.0);

    // Score at exactly 98% of baseline
    let at_threshold = baseline * 0.98;
    assert!(
        at_threshold >= min_acceptable,
        "exact threshold should pass: {at_threshold} vs {min_acceptable}"
    );

    // Score below 98% should fail
    let below_threshold = baseline * 0.979;
    assert!(
        below_threshold < min_acceptable,
        "below threshold should fail: {below_threshold} vs {min_acceptable}"
    );
}
