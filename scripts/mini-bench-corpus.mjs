import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

export const CORPUS_DIR = "/tmp/quaid-bench-corpus";

const __filename = fileURLToPath(import.meta.url);
export const __dirname = path.dirname(__filename);

function hasMarkdownFiles(dir) {
  if (!fs.existsSync(dir)) return false;

  const entries = fs.readdirSync(dir, { withFileTypes: true });
  for (const entry of entries) {
    const entryPath = path.join(dir, entry.name);
    if (entry.isFile() && entry.name.endsWith(".md")) return true;
    if (entry.isDirectory() && hasMarkdownFiles(entryPath)) return true;
  }

  return false;
}

export function resolveCorpusDir(corpusDir = CORPUS_DIR) {
  if (!hasMarkdownFiles(corpusDir)) {
    throw new Error(
      `DAB corpus not found at ${corpusDir}.\nRun: cd ~/repos/quaid-evals && python3 benchmarks/dab/generate_corpus.py`
    );
  }

  return corpusDir;
}

if (process.argv[1] && path.resolve(process.argv[1]) === __filename) {
  const corpusDir = resolveCorpusDir(process.argv[2] || CORPUS_DIR);
  console.log(`Using DAB corpus at ${corpusDir}`);
}
