import { spawnSync } from "node:child_process";
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { CORPUS_DIR, generateCorpus } from "./mini-bench-corpus.mjs";

const __filename = fileURLToPath(import.meta.url);
const __dirname = path.dirname(__filename);
const REPO_DIR = path.dirname(__dirname);
const DEFAULT_DB = "/tmp/quaid-mini-bench.db";
const DEFAULT_QUAID = path.join(REPO_DIR, "target", "release", "quaid");

function parseArgs(argv) {
  const args = {
    db: DEFAULT_DB,
    quaid: DEFAULT_QUAID,
  };

  for (let i = 0; i < argv.length; i += 1) {
    const arg = argv[i];
    if (arg === "--db") {
      args.db = argv[++i];
    } else if (arg === "--quaid") {
      args.quaid = argv[++i];
    } else {
      throw new Error(`unknown argument: ${arg}`);
    }
  }

  return args;
}

function run(quaid, args, options = {}) {
  const result = spawnSync(quaid, args, {
    cwd: REPO_DIR,
    encoding: "utf8",
    stdio: ["ignore", "pipe", "pipe"],
    ...options,
  });

  return result;
}

function fail(step, result) {
  const detail = [result.stdout, result.stderr].filter(Boolean).join("\n").trim();
  console.error(`${step} failed${detail ? `:\n${detail}` : ""}`);
  process.exit(result.status || 1);
}

const args = parseArgs(process.argv.slice(2));
const quaid = path.resolve(args.quaid);

if (!fs.existsSync(quaid)) {
  console.error(`ERROR: quaid binary not found at ${quaid}`);
  console.error("Run: cargo build --release");
  process.exit(1);
}

console.log(`Generating corpus at ${CORPUS_DIR}...`);
generateCorpus(CORPUS_DIR);

if (!fs.existsSync(args.db)) {
  console.log(`Initialising DB at ${args.db}...`);
  const init = run(quaid, ["init", args.db]);
  if (init.status !== 0) fail("init", init);
} else {
  console.log(`Using existing DB at ${args.db}...`);
}

console.log(`Adding corpus collection from ${CORPUS_DIR}...`);
const add = run(quaid, ["--db", args.db, "collection", "add", "docs", CORPUS_DIR]);
if (add.status !== 0) {
  const output = `${add.stdout}\n${add.stderr}`;
  if (!/already exists|collection exists|duplicate/i.test(output)) {
    fail("collection add", add);
  }
  console.log("Collection docs already exists; continuing.");
}

console.log("Syncing collection...");
const sync = run(quaid, ["--db", args.db, "collection", "sync", "docs"]);
if (sync.status !== 0) fail("collection sync", sync);

console.log("Generating embeddings...");
const embed = run(quaid, ["--db", args.db, "embed", "--all"], { timeout: 300_000 });
if (embed.status !== 0) fail("embed", embed);

console.log("Ready.");
