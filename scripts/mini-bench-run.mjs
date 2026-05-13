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
  { query: "memory systems persistent storage sessions archive 2025", expected: "2025/page-0089", label: "S01" },
  { query: "semantic search dense vector representations health area", expected: "health/page-0038", label: "S02" },
  { query: "regulatory clarity institutional adoption AI tools resource", expected: "ai-tools/page-0051", label: "S03" },
  { query: "formal verification exploit surface 2024 archive part 7", expected: "2024/page-0083", label: "S04" },
  { query: "automated market maker pooled reserves", expected: "defi", label: "S05" },
  { query: "stable asset issuer redemption reserves", expected: "stablecoin", label: "S06" },
  { query: "Projects Areas Resources Archives finance area part 7", expected: "finance/page-0034", label: "S07" },
  { query: "RAG generative models research resource part 3", expected: "research/page-0072", label: "S08" },
  { query: "zero copy request latency tuning", expected: "rust", label: "S09" },
  { query: "Bitcoin 75000 valuation supply velocity utility", expected: "token", label: "S10" },
];

const SEMANTIC_QUERIES = [
  { query: "Which project gamma note is about AI agents retaining state between sessions?", expected: "project-gamma/page-0020", label: "Q01" },
  { query: "Find the 2025 archive note about graph structures modeling relationships naturally.", expected: "2025/page-0084", label: "Q02" },
  { query: "Where is the Dev Tools resource about formal verification reducing the DeFi exploit surface?", expected: "dev-tools/page-0069", label: "Q03" },
  { query: "Which finance area note ties PARA organization to stablecoins, DeFi liquidity, and graph databases?", expected: "finance/page-0034", label: "Q04" },
  { query: "Find the learning area document on AMM liquidity pools with stablecoin regulation as a related topic.", expected: "learning/page-0046", label: "Q05" },
  { query: "Which page discusses stablecoin clarity for institutions in project beta part 3?", expected: "project-beta/page-0009", label: "Q06" },
  { query: "Which AI tools retrieval generation note part 7 is connected to stablecoin regulation?", expected: "ai-tools/page-0055", label: "Q07" },
  { query: "Find the project alpha PARA page that mentions smart contract auditing and agent memory in key points.", expected: "project-alpha/page-0000", label: "Q08" },
  { query: "Which learning area Rust performance note part 3 has graph databases and DeFi as key points?", expected: "learning/page-0044", label: "Q09" },
  { query: "Where is the research smart contract auditing part 2 note connected to stablecoin and token economics?", expected: "research/page-0071", label: "Q10" },
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
