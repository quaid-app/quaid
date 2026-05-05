use std::env;
use std::time::Instant;

use quaid::core::conversation::extractor::{PendingFactWriter, Worker};
use quaid::core::conversation::model_lifecycle::load_model_from_local_cache;
use quaid::core::conversation::slm::LazySlmRunner;
use quaid::core::db;
use quaid::core::types::{Turn, TurnRole, WindowedTurns};

const DEFAULT_MODEL_ALIAS: &str = "phi-3.5-mini";
const DEFAULT_SAMPLES: usize = 20;
const DEFAULT_LOOKBACK: usize = 3;
fn main() {
    if let Some(reason) = skip_reason() {
        println!("SKIPPED: {reason}");
        return;
    }

    let model_alias = env::var("QUAID_EXTRACTION_BENCH_MODEL_ALIAS")
        .ok()
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(|| DEFAULT_MODEL_ALIAS.to_string());
    let threshold_ms = threshold_ms();
    let samples = env::var("QUAID_EXTRACTION_BENCH_SAMPLES")
        .ok()
        .and_then(|value| value.parse::<usize>().ok())
        .filter(|value| *value > 0)
        .unwrap_or(DEFAULT_SAMPLES);

    if let Err(error) = load_model_from_local_cache(&model_alias) {
        eprintln!(
            "Representative-hardware extraction bench requires a staged local model cache for `{model_alias}`: {error}"
        );
        std::process::exit(1);
    }

    let tempdir = tempfile::TempDir::new().expect("benchmark tempdir");
    let db_path = tempdir.path().join("memory.db");
    let conn = db::open(db_path.to_str().expect("db path")).expect("open benchmark db");
    conn.execute(
        "INSERT OR REPLACE INTO config(key, value) VALUES
            ('extraction.model_alias', ?1),
            ('extraction.enabled', 'true')",
        [model_alias.as_str()],
    )
    .expect("configure benchmark db");

    let worker = Worker::new(&conn, LazySlmRunner::new(), PendingFactWriter)
        .expect("construct extraction worker");
    let windows = benchmark_windows(samples);

    let warmup = worker
        .infer_window("bench-warmup", &windows[0])
        .expect("warm model with one extraction window");
    println!(
        "Warm-up complete with {} extracted facts.",
        warmup.facts.len()
    );

    let mut durations_ms = Vec::with_capacity(windows.len());
    for (index, window) in windows.iter().enumerate() {
        let started = Instant::now();
        let response = worker
            .infer_window(&format!("bench-session-{index}"), window)
            .expect("benchmark extraction window");
        let elapsed_ms = started.elapsed().as_secs_f64() * 1000.0;
        durations_ms.push(elapsed_ms);
        println!(
            "sample {:02}: {:7.1} ms (facts={})",
            index + 1,
            elapsed_ms,
            response.facts.len()
        );
    }

    durations_ms.sort_by(|left, right| left.total_cmp(right));
    let p50 = percentile(&durations_ms, 50);
    let p95 = percentile(&durations_ms, 95);

    println!(
        "Extraction benchmark ({}/{}) -> p50 {:.1} ms, p95 {:.1} ms, threshold {:.1} ms",
        env::consts::OS,
        env::consts::ARCH,
        p50,
        p95,
        threshold_ms
    );

    if p95 >= threshold_ms {
        eprintln!(
            "extraction p95 {p95:.1}ms exceeds representative-hardware gate of {threshold_ms:.1}ms"
        );
        std::process::exit(1);
    }
}

fn skip_reason() -> Option<String> {
    if env::var("QUAID_EXTRACTION_BENCH_FORCE")
        .map(|value| value == "1")
        .unwrap_or(false)
    {
        return None;
    }

    let supported = matches!(
        (env::consts::OS, env::consts::ARCH),
        ("macos", "aarch64") | ("linux", "x86_64")
    );
    if supported {
        None
    } else {
        Some(format!(
            "representative-hardware gate only applies on macOS/aarch64 and linux/x86_64; got {}/{}",
            env::consts::OS,
            env::consts::ARCH
        ))
    }
}

fn threshold_ms() -> f64 {
    match (env::consts::OS, env::consts::ARCH) {
        ("macos", "aarch64") => 3_000.0,
        ("linux", "x86_64") => 8_000.0,
        _ => env::var("QUAID_EXTRACTION_BENCH_THRESHOLD_MS")
            .ok()
            .and_then(|value| value.parse::<f64>().ok())
            .unwrap_or(8_000.0),
    }
}

fn percentile(sorted: &[f64], pct: usize) -> f64 {
    let index = (pct * (sorted.len() - 1)).div_ceil(100);
    sorted[index]
}

fn benchmark_windows(samples: usize) -> Vec<WindowedTurns> {
    (0..samples).map(representative_window).collect()
}

fn representative_window(index: usize) -> WindowedTurns {
    let topic = format!("project-cinder-{index}");
    let lookback_turns = (0..DEFAULT_LOOKBACK)
        .map(|ordinal| {
            turn(
                ordinal as i64 + 1,
                if ordinal % 2 == 0 {
                    TurnRole::User
                } else {
                    TurnRole::Assistant
                },
                format!("2026-05-05T08:{:02}:00Z", (index * 2 + ordinal) % 60),
                match ordinal {
                    0 => format!("We are planning the rollout for {topic}."),
                    1 => format!("I will track the migration and release notes for {topic}."),
                    _ => format!("Keep the notes concise and local-first for {topic}."),
                },
            )
        })
        .collect();

    let new_turns = vec![
        turn(
            10,
            TurnRole::User,
            "2026-05-05T09:00:00Z".to_string(),
            format!(
                "Decision for {topic}: use SQLite for the local cache and keep extraction fully local."
            ),
        ),
        turn(
            11,
            TurnRole::Assistant,
            "2026-05-05T09:01:00Z".to_string(),
            format!("Captured: SQLite is the cache choice for {topic}."),
        ),
        turn(
            12,
            TurnRole::User,
            "2026-05-05T09:02:00Z".to_string(),
            format!("Preference for {topic}: favor Rust services over Python when performance matters."),
        ),
        turn(
            13,
            TurnRole::Assistant,
            "2026-05-05T09:03:00Z".to_string(),
            format!("Noted. Rust is the preferred implementation path for {topic}."),
        ),
        turn(
            14,
            TurnRole::User,
            "2026-05-05T09:04:00Z".to_string(),
            format!("Action item: Alex should publish the {topic} rollout checklist by Friday."),
        ),
    ];

    WindowedTurns {
        lookback_turns,
        new_turns,
        context_only: false,
    }
}

fn turn(ordinal: i64, role: TurnRole, timestamp: String, content: String) -> Turn {
    Turn {
        ordinal,
        role,
        timestamp,
        content,
        metadata: None,
    }
}
