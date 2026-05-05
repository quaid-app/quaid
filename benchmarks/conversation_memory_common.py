#!/usr/bin/env python3
"""
Shared helpers for conversation-memory benchmark runs.

These helpers exercise the real conversation pipeline:
memory_add_turn → memory_close_session → extraction worker → fact-page query.
"""

from __future__ import annotations

import json
import os
import sqlite3
import subprocess
import sys
import time
from contextlib import contextmanager
from datetime import datetime, timedelta, timezone
from pathlib import Path
from typing import Any, Iterator

REPO_ROOT = Path(__file__).parent.parent
QUAID_BIN = os.environ.get("QUAID_BIN", str(REPO_ROOT / "target" / "release" / "quaid"))
FACT_PAGE_TYPES = {"decision", "preference", "fact", "action_item"}
VALID_TURN_ROLES = {"user", "assistant", "system", "tool"}


class ConversationBenchmarkError(RuntimeError):
    """Raised when the benchmark harness cannot exercise the real pipeline."""


def ensure_quaid_bin() -> None:
    if not Path(QUAID_BIN).exists():
        sys.exit(f"quaid binary not found at {QUAID_BIN}. Run: cargo build --release")


def ensure_unix_collection_support() -> None:
    if os.name == "nt":
        sys.exit(
            "Conversation-memory benchmarks require Unix because "
            "`quaid collection add` is the truthful writable-root path today."
        )


def run_quaid(
    args: list[str],
    *,
    check: bool = True,
    capture_output: bool = True,
) -> subprocess.CompletedProcess[str]:
    result = subprocess.run(
        [QUAID_BIN, *args],
        cwd=REPO_ROOT,
        text=True,
        capture_output=capture_output,
    )
    if check and result.returncode != 0:
        stderr = result.stderr.strip()
        stdout = result.stdout.strip()
        detail = stderr or stdout or f"exit code {result.returncode}"
        raise ConversationBenchmarkError(
            f"quaid command failed: {' '.join(args)}\n{detail}"
        )
    return result


def run_quaid_json(args: list[str]) -> Any:
    result = run_quaid(args)
    text = result.stdout.strip()
    if not text:
        return None
    try:
        return json.loads(text)
    except json.JSONDecodeError as exc:
        raise ConversationBenchmarkError(
            f"Expected JSON from `quaid {' '.join(args)}`, got:\n{text}"
        ) from exc


def call_tool(db_path: str, tool: str, params: dict[str, Any]) -> Any:
    return run_quaid_json(["--db", db_path, "call", tool, json.dumps(params)])


def configure_workspace(
    db_path: str,
    workspace_dir: Path,
    *,
    model_alias: str | None = None,
) -> Path:
    ensure_quaid_bin()
    ensure_unix_collection_support()

    workspace_dir.mkdir(parents=True, exist_ok=True)
    vault_dir = workspace_dir / "vault"
    vault_dir.mkdir(parents=True, exist_ok=True)

    run_quaid(["--db", db_path, "init"])
    if model_alias:
        run_quaid(["--db", db_path, "config", "set", "extraction.model_alias", model_alias])
    run_quaid(["--db", db_path, "collection", "add", "bench", str(vault_dir), "--writable"])
    run_quaid(["--db", db_path, "extraction", "enable"])
    return vault_dir


def _read_sqlite_one(
    db_path: str,
    sql: str,
    params: tuple[Any, ...] = (),
) -> Any:
    with sqlite3.connect(db_path, timeout=30) as conn:
        return conn.execute(sql, params).fetchone()


def extraction_counts(db_path: str) -> dict[str, int]:
    row = _read_sqlite_one(
        db_path,
        """
        SELECT
            COALESCE(SUM(CASE WHEN status = 'pending' THEN 1 ELSE 0 END), 0),
            COALESCE(SUM(CASE WHEN status = 'running' THEN 1 ELSE 0 END), 0),
            COALESCE(SUM(CASE WHEN status = 'done' THEN 1 ELSE 0 END), 0),
            COALESCE(SUM(CASE WHEN status = 'failed' THEN 1 ELSE 0 END), 0)
        FROM extraction_queue
        """,
    )
    pending, running, done, failed = row or (0, 0, 0, 0)
    return {
        "pending": int(pending),
        "running": int(running),
        "done": int(done),
        "failed": int(failed),
    }


def recent_failed_jobs(db_path: str) -> list[dict[str, Any]]:
    with sqlite3.connect(db_path, timeout=30) as conn:
        conn.row_factory = sqlite3.Row
        rows = conn.execute(
            """
            SELECT session_id, attempts, COALESCE(last_error, '') AS last_error
            FROM extraction_queue
            WHERE status = 'failed'
            ORDER BY id DESC
            LIMIT 5
            """
        ).fetchall()
    return [dict(row) for row in rows]


@contextmanager
def serve_runtime(db_path: str) -> Iterator[subprocess.Popen[str]]:
    ensure_quaid_bin()
    process = subprocess.Popen(
        [QUAID_BIN, "--db", db_path, "serve"],
        cwd=REPO_ROOT,
        stdin=subprocess.PIPE,
        stdout=subprocess.DEVNULL,
        stderr=subprocess.PIPE,
        text=True,
    )
    time.sleep(2.0)
    if process.poll() is not None:
        stderr = ""
        if process.stderr is not None:
            stderr = process.stderr.read().strip()
        raise ConversationBenchmarkError(
            f"`quaid serve` exited before the benchmark started.\n{stderr}"
        )
    try:
        yield process
    finally:
        if process.poll() is None:
            process.terminate()
            try:
                process.wait(timeout=10)
            except subprocess.TimeoutExpired:
                process.kill()
                process.wait(timeout=10)


def wait_for_extraction_completion(
    db_path: str,
    *,
    timeout_s: int = 900,
    poll_interval_s: float = 0.5,
    settle_s: float = 2.0,
) -> dict[str, int]:
    deadline = time.time() + timeout_s
    while time.time() < deadline:
        counts = extraction_counts(db_path)
        if counts["failed"] > 0:
            failed = recent_failed_jobs(db_path)
            raise ConversationBenchmarkError(
                "Extraction worker reported failed jobs: "
                + json.dumps(failed, indent=2)
            )
        if counts["pending"] == 0 and counts["running"] == 0:
            time.sleep(settle_s)
            return extraction_counts(db_path)
        time.sleep(poll_interval_s)
    raise ConversationBenchmarkError(
        f"Timed out waiting for extraction queue to drain: {extraction_counts(db_path)}"
    )


def fact_page_count(db_path: str) -> int:
    placeholders = ",".join("?" for _ in FACT_PAGE_TYPES)
    row = _read_sqlite_one(
        db_path,
        f"SELECT COUNT(*) FROM pages WHERE type IN ({placeholders})",
        tuple(sorted(FACT_PAGE_TYPES)),
    )
    return int((row or (0,))[0])


def fact_pages(db_path: str, slugs: list[str]) -> list[dict[str, str]]:
    pages: list[dict[str, str]] = []
    with sqlite3.connect(db_path, timeout=30) as conn:
        conn.row_factory = sqlite3.Row
        for slug in slugs:
            row = conn.execute(
                """
                SELECT slug, type, summary, compiled_truth
                FROM pages
                WHERE slug = ?1
                """,
                (slug,),
            ).fetchone()
            if row is None:
                continue
            pages.append(dict(row))
    return pages


def ranked_slugs(
    db_path: str,
    query: str,
    *,
    mode: str,
    limit: int = 20,
) -> list[str]:
    if mode == "hybrid":
        results = run_quaid_json(
            ["--db", db_path, "--json", "query", query, "--limit", str(limit)]
        )
    elif mode == "fts5":
        results = run_quaid_json(
            ["--db", db_path, "--json", "search", query, "--limit", str(limit)]
        )
    else:
        raise ValueError(f"unsupported retrieval mode: {mode}")
    if not isinstance(results, list):
        return []
    return [item.get("slug", "") for item in results if item.get("slug")]


def ranked_fact_slugs(
    db_path: str,
    query: str,
    *,
    mode: str,
    k: int = 5,
    limit: int = 20,
) -> list[str]:
    selected: list[str] = []
    for page in fact_pages(db_path, ranked_slugs(db_path, query, mode=mode, limit=limit)):
        if page.get("type") not in FACT_PAGE_TYPES:
            continue
        selected.append(page["slug"])
        if len(selected) >= k:
            break
    return selected


def ranked_fact_texts(
    db_path: str,
    query: str,
    *,
    mode: str,
    k: int = 5,
    limit: int = 20,
) -> list[str]:
    texts: list[str] = []
    for page in fact_pages(db_path, ranked_slugs(db_path, query, mode=mode, limit=limit)):
        if page.get("type") not in FACT_PAGE_TYPES:
            continue
        compiled_truth = page.get("compiled_truth", "").strip()
        summary = page.get("summary", "").strip()
        text = " ".join(part for part in (compiled_truth, summary) if part).strip()
        if text:
            texts.append(text)
        if len(texts) >= k:
            break
    return texts


def normalize_answer(answer: Any) -> str:
    if isinstance(answer, str):
        return answer.strip()
    if isinstance(answer, list):
        return " ".join(normalize_answer(item) for item in answer if normalize_answer(item))
    if answer is None:
        return ""
    return str(answer).strip()


def token_f1(prediction: str, ground_truth: str) -> float:
    pred_tokens = set(prediction.lower().split())
    truth_tokens = set(ground_truth.lower().split())
    if not pred_tokens or not truth_tokens:
        return 0.0
    common = pred_tokens & truth_tokens
    precision = len(common) / len(pred_tokens)
    recall = len(common) / len(truth_tokens)
    if precision + recall == 0:
        return 0.0
    return 2 * precision * recall / (precision + recall)


def max_f1(retrieved_texts: list[str], ground_truth: str) -> float:
    if not retrieved_texts:
        return 0.0
    return max(token_f1(text, ground_truth) for text in retrieved_texts)


def answer_hit_at_k(
    retrieved_texts: list[str],
    ground_truth: str,
    *,
    f1_threshold: float = 0.5,
) -> float:
    return 1.0 if max_f1(retrieved_texts, ground_truth) >= f1_threshold else 0.0


def coerce_role(raw_role: str | None, speaker_map: dict[str, str]) -> str:
    role = (raw_role or "").strip().lower()
    if role in VALID_TURN_ROLES:
        return role
    if any(token in role for token in ("assistant", "bot", "agent")):
        return "assistant"
    if "system" in role:
        return "system"
    if "tool" in role:
        return "tool"
    if role not in speaker_map:
        speaker_map[role] = "user" if not speaker_map else "assistant"
    return speaker_map[role]


def normalize_timestamp(value: Any, *, session_index: int, turn_index: int) -> str:
    if isinstance(value, str):
        candidate = value.strip()
        if candidate:
            normalized = candidate.replace("Z", "+00:00")
            try:
                datetime.fromisoformat(normalized)
                return candidate if candidate.endswith("Z") else normalized.replace("+00:00", "Z")
            except ValueError:
                pass
    base = datetime(2026, 1, 1, tzinfo=timezone.utc) + timedelta(days=session_index)
    timestamp = base + timedelta(minutes=turn_index)
    return timestamp.isoformat().replace("+00:00", "Z")


def ingest_turns(
    db_path: str,
    session_id: str,
    turns: list[dict[str, Any]],
    *,
    namespace: str | None = None,
    session_index: int = 0,
) -> None:
    for turn_index, turn in enumerate(turns, start=1):
        params = {
            "session_id": session_id,
            "role": turn["role"],
            "content": turn["content"],
            "timestamp": normalize_timestamp(
                turn.get("timestamp"),
                session_index=session_index,
                turn_index=turn_index,
            ),
        }
        if namespace:
            params["namespace"] = namespace
        call_tool(db_path, "memory_add_turn", params)
    close_params: dict[str, Any] = {"session_id": session_id}
    if namespace:
        close_params["namespace"] = namespace
    call_tool(db_path, "memory_close_session", close_params)
