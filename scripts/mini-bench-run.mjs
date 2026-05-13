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
  { query: "agent memory architecture", expected: ["agent-memory", "Agent Memory"], label: "S01" },
  { query: "stablecoin regulation", expected: ["stablecoin", "Stablecoin"], label: "S02" },
  { query: "DeFi liquidity protocol", expected: ["defi", "DeFi"], label: "S03" },
  { query: "Rust async runtime performance", expected: ["rust", "Rust"], label: "S04" },
  { query: "vector embedding search", expected: ["vector", "Vector"], label: "S05" },
  { query: "knowledge graph traversal", expected: ["knowledge-graph", "Knowledge Graph"], label: "S06" },
  { query: "smart contract security audit", expected: ["smart-contract", "Smart Contract"], label: "S07" },
  { query: "cross-chain bridge mechanism", expected: ["cross-chain", "Cross-Chain"], label: "S08" },
  { query: "zero knowledge proof circuit", expected: ["zero-knowledge", "Zero Knowledge"], label: "S09" },
  { query: "retrieval augmented generation", expected: ["rag", "Retrieval"], label: "S10" },
];

const SEMANTIC_QUERIES = [
  { query: "how do AI agents remember information across multiple sessions", expected: ["agent-memory"], label: "Q01" },
  { query: "what makes decentralized exchanges different from centralized ones", expected: ["defi"], label: "Q02" },
  { query: "techniques for compressing large language model context windows", expected: ["rag", "Retrieval"], label: "Q03" },
  { query: "regulatory challenges for stablecoin issuers in the US", expected: ["stablecoin"], label: "Q04" },
  { query: "building efficient search over large markdown document collections", expected: ["vector"], label: "Q05" },
  { query: "how to detect contradictions in a knowledge base", expected: ["knowledge-graph", "Knowledge"], label: "Q06" },
  { query: "performance optimisation strategies for Rust web services", expected: ["rust", "Rust"], label: "Q07" },
  { query: "blockchain interoperability and cross-chain asset transfers", expected: ["cross-chain"], label: "Q08" },
  { query: "cryptographic commitments and privacy in smart contracts", expected: ["zero-knowledge", "Zero"], label: "Q09" },
  { query: "graph-based reasoning and link traversal in document stores", expected: ["knowledge-graph"], label: "Q10" },
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
  return results.some((result) => {
    const haystack = `${result.slug || ""} ${result.title || ""}`.toLowerCase();
    return expected.some((needle) => haystack.includes(needle.toLowerCase()));
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
