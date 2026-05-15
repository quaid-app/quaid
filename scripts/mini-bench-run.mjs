import { spawnSync } from "node:child_process";
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

const __filename = fileURLToPath(import.meta.url);
const __dirname = path.dirname(__filename);
const REPO_DIR = path.dirname(__dirname);
const DEFAULT_DB = "/tmp/quaid-mini-bench.db";
const DEFAULT_QUAID = path.join(REPO_DIR, "target", "release", "quaid");
const DEFAULT_PREV = "/tmp/quaid-mini-bench-prev.txt";

const FTS_QUERIES = [
  { query: "agent memory architecture persistent storage sessions", expected: ["Agent Memory", "agent-memory", "page-0089"], label: "S01" },
  { query: "vector embeddings semantic search dense representations", expected: ["Vector Embeddings", "vector-embeddings", "vector"], label: "S02" },
  { query: "stablecoin regulation clarity institutional adoption", expected: ["Stablecoin Regulation", "stablecoin-regulation", "stablecoin"], label: "S03" },
  { query: "Rust async runtime performance optimization", expected: ["Rust Performance", "rust-performance", "rust"], label: "S04" },
  { query: "automated market maker DeFi liquidity protocol pooled reserves", expected: ["DeFi Liquidity", "defi-liquidity", "defi"], label: "S05" },
  { query: "knowledge graph traversal entity relationships", expected: ["Knowledge Graph", "knowledge-graph", "knowledge"], label: "S06" },
  { query: "smart contract security audit formal verification exploit", expected: ["Smart Contract", "smart-contract", "smart"], label: "S07" },
  { query: "cross-chain bridge interoperability asset transfer", expected: ["Cross-Chain", "cross-chain", "cross"], label: "S08" },
  { query: "zero knowledge proof cryptographic circuit commitment", expected: ["Zero Knowledge", "zero-knowledge", "zero"], label: "S09" },
  { query: "retrieval augmented generation RAG document search", expected: ["Retrieval Augmented", "rag-retrieval", "retrieval"], label: "S10" },
];

const SEMANTIC_QUERIES = [
  { query: "how do AI agents remember information across multiple sessions", expected: ["Agent Memory", "agent-memory", "page-0084", "page-0089", "page-0020"], label: "Q01" },
  { query: "what makes decentralized exchanges different from centralized ones", expected: ["DeFi", "defi-liquidity", "defi", "page-0046"], label: "Q02" },
  { query: "techniques for compressing large language model context windows", expected: ["Retrieval", "rag-retrieval", "retrieval", "page-0072"], label: "Q03" },
  { query: "regulatory challenges for stablecoin issuers in the US", expected: ["Stablecoin", "stablecoin-regulation", "stablecoin", "page-0009"], label: "Q04" },
  { query: "building efficient search over large markdown document collections", expected: ["Vector Embeddings", "vector-embeddings", "vector", "page-0038"], label: "Q05" },
  { query: "how to detect contradictions in a knowledge base graph", expected: ["Knowledge Graph", "knowledge-graph", "knowledge", "page-0034"], label: "Q06" },
  { query: "performance optimisation strategies for Rust web services", expected: ["Rust Performance", "rust-performance", "rust", "page-0044"], label: "Q07" },
  { query: "blockchain interoperability and cross-chain asset transfers", expected: ["Cross-Chain", "cross-chain", "cross", "page-0000"], label: "Q08" },
  { query: "cryptographic commitments and privacy in smart contracts", expected: ["Zero Knowledge", "zero-knowledge", "zero", "page-0055"], label: "Q09" },
  { query: "graph-based reasoning and link traversal in document stores", expected: ["Knowledge Graph", "knowledge-graph", "knowledge", "page-0071"], label: "Q10" },
];

function parseArgs(argv) {
  const args = {
    db: DEFAULT_DB,
    quaid: DEFAULT_QUAID,
    prev: DEFAULT_PREV,
  };

  for (let i = 0; i < argv.length; i += 1) {
    const arg = argv[i];
    if (arg === "--db") {
      args.db = argv[++i];
    } else if (arg === "--quaid") {
      args.quaid = argv[++i];
    } else if (arg === "--prev") {
      args.prev = argv[++i];
    } else {
      throw new Error(`unknown argument: ${arg}`);
    }
  }

  return args;
}

function runQuery(quaid, db, command, query) {
  const result = spawnSync(quaid, ["--db", db, command, query, "--limit", "5", "--json"], {
    cwd: REPO_DIR,
    encoding: "utf8",
    stdio: ["ignore", "pipe", "pipe"],
    timeout: 15_000,
  });

  if (result.status !== 0) {
    return [];
  }

  try {
    const data = JSON.parse(result.stdout);
    return Array.isArray(data) ? data : data.results || [];
  } catch {
    return [];
  }
}

function checkHit(results, expected) {
  const needles = Array.isArray(expected) ? expected : [expected];
  return results.some((result) => {
    const haystack = `${result.slug || ""} ${result.title || ""}`.toLowerCase();
    return needles.some((needle) => haystack.includes(needle.toLowerCase()));
  });
}

function bar(n, total, width = 10) {
  const filled = Math.round((n / total) * width);
  return `${"█".repeat(filled)}${"░".repeat(width - filled)}`;
}

function formatLabels(results) {
  return results.map(({ label, passed }) => `${label}${passed ? "✓" : "✗"}`).join(" ");
}

function readPrev(prevPath) {
  try {
    const raw = fs.readFileSync(prevPath, "utf8").trim();
    const parsed = Number.parseInt(raw, 10);
    return Number.isNaN(parsed) ? null : parsed;
  } catch {
    return null;
  }
}

const args = parseArgs(process.argv.slice(2));
const quaid = path.resolve(args.quaid);

if (!fs.existsSync(quaid)) {
  console.error(`ERROR: quaid binary not found at ${quaid}`);
  console.error("Run: cargo build --release");
  process.exit(1);
}

if (!fs.existsSync(args.db)) {
  console.error(`ERROR: bench DB not found at ${args.db}`);
  console.error("Run: node scripts/mini-bench-setup.mjs");
  process.exit(1);
}

const prevScore = readPrev(args.prev);
const start = performance.now();

const ftsResults = FTS_QUERIES.map((item) => {
  const results = runQuery(quaid, args.db, "search", item.query);
  return { label: item.label, passed: checkHit(results, item.expected) };
});

const semanticResults = SEMANTIC_QUERIES.map((item) => {
  const results = runQuery(quaid, args.db, "query", item.query);
  return { label: item.label, passed: checkHit(results, item.expected) };
});

const elapsedSeconds = (performance.now() - start) / 1000;
const ftsPass = ftsResults.filter((result) => result.passed).length;
const semanticPass = semanticResults.filter((result) => result.passed).length;
const total = ftsPass + semanticPass;
const maxTotal = 20;
const grade = total >= 14 ? "✅" : total >= 10 ? "⚠️" : "🔴";

let delta = "";
if (prevScore !== null) {
  const diff = total - prevScore;
  delta = `  prev: ${prevScore}/${maxTotal}  delta: ${diff >= 0 ? "+" : ""}${diff}`;
}

console.log("");
console.log(`§3 FTS  [${bar(ftsPass, 10)}] ${ftsPass}/10   ${formatLabels(ftsResults)}`);
console.log(`§4 Sem  [${bar(semanticPass, 10)}] ${semanticPass}/10   ${formatLabels(semanticResults)}`);
console.log(`Total: ${total}/${maxTotal}${delta}  (${elapsedSeconds.toFixed(1)}s)  ${grade}`);
console.log("");

try {
  fs.writeFileSync(args.prev, String(total), "utf8");
} catch {
  // Best-effort delta tracking should not fail the benchmark.
}

if (total < 8) {
  console.error("🔴 REGRESSION: score below gate (8/20)");
  process.exit(1);
}
