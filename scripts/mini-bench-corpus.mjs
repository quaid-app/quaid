import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

export const CORPUS_DIR = "/tmp/quaid-bench-corpus";

const __filename = fileURLToPath(import.meta.url);
export const __dirname = path.dirname(__filename);

const TOPICS = [
  {
    tag: "agent-memory",
    title: "Agent Memory Architecture",
    sentences: [
      "Agent memory architecture defines how autonomous agents store durable working context, retrieved facts, and long-term preferences.",
      "This note covers agent-memory keywords such as episodic memory, semantic memory, memory consolidation, and retrieval policy.",
      "The architecture separates current compiled knowledge from append-only evidence so agents can update beliefs without losing provenance.",
    ],
  },
  {
    tag: "stablecoin-regulation",
    title: "Stablecoin Regulation",
    sentences: [
      "Stablecoin regulation focuses on reserve quality, issuer supervision, redemption rights, and disclosure requirements for payment stablecoins.",
      "This stablecoin-regulation note mentions keywords such as reserves, attestations, prudential oversight, and consumer protection.",
      "Regulators evaluate whether stablecoin issuers can maintain one-to-one backing while supporting transparent market operations.",
    ],
  },
  {
    tag: "defi-liquidity",
    title: "DeFi Liquidity Protocol",
    sentences: [
      "DeFi liquidity protocols coordinate market makers, liquidity pools, incentives, and automated pricing across decentralized exchanges.",
      "This defi-liquidity note uses keywords such as AMM, liquidity mining, impermanent loss, total value locked, and pool depth.",
      "Protocol design balances capital efficiency with risk controls so liquidity providers understand exposure during volatile markets.",
    ],
  },
  {
    tag: "rust-performance",
    title: "Rust Performance",
    sentences: [
      "Rust performance work studies allocation patterns, ownership boundaries, async scheduling, and cache-friendly data layouts.",
      "This rust-performance note mentions keywords such as zero-cost abstractions, borrow checking, SIMD, profiling, and latency.",
      "Careful measurement helps Rust services improve throughput without sacrificing safety or maintainability.",
    ],
  },
  {
    tag: "vector-embeddings",
    title: "Vector Embeddings",
    sentences: [
      "Vector embeddings represent documents, queries, and entities as dense numeric vectors for semantic similarity search.",
      "This vector-embeddings note includes keywords such as cosine similarity, embedding models, dimensions, indexing, and recall.",
      "Embedding quality affects whether nearby vectors capture the concepts users expect during retrieval and clustering.",
    ],
  },
  {
    tag: "knowledge-graph",
    title: "Knowledge Graph",
    sentences: [
      "A knowledge graph stores entities, relationships, assertions, and evidence as connected records that can be queried by path.",
      "This knowledge-graph note mentions keywords such as nodes, edges, provenance, ontology, link prediction, and graph traversal.",
      "Graph-aware retrieval can explain why related pages matter by showing relationship paths between concepts.",
    ],
  },
  {
    tag: "smart-contract",
    title: "Smart Contract Security",
    sentences: [
      "Smart contract security examines code paths that control assets, permissions, upgrades, and protocol invariants on-chain.",
      "This smart-contract note uses keywords such as reentrancy, formal verification, audit findings, access control, and exploits.",
      "Secure contracts combine defensive design, testing, review, and monitoring to reduce loss from irreversible transactions.",
    ],
  },
  {
    tag: "cross-chain",
    title: "Cross-Chain Bridge",
    sentences: [
      "A cross-chain bridge moves messages or assets between blockchains using validators, light clients, or liquidity networks.",
      "This cross-chain note mentions bridge keywords such as finality, relayers, wrapped assets, interoperability, and trust assumptions.",
      "Bridge architecture must account for chain reorganizations, proof verification, and failure modes across independent networks.",
    ],
  },
  {
    tag: "zero-knowledge",
    title: "Zero Knowledge Proof",
    sentences: [
      "Zero knowledge proof systems let a prover convince a verifier that a statement is true without revealing private inputs.",
      "This zero-knowledge note includes keywords such as zk-SNARK, zk-STARK, circuits, commitments, witnesses, and verification.",
      "Applications use zero knowledge proofs for privacy, scalability, identity, and succinct verification of computation.",
    ],
  },
  {
    tag: "rag-retrieval",
    title: "Retrieval Augmented Generation",
    sentences: [
      "Retrieval augmented generation combines search results with model prompts so answers are grounded in selected documents.",
      "This rag-retrieval note mentions keywords such as chunking, reranking, hybrid search, citations, context windows, and answer synthesis.",
      "A strong RAG retrieval pipeline improves factuality by selecting relevant evidence before generation begins.",
    ],
  },
];

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

export function generateCorpus(corpusDir) {
  fs.mkdirSync(corpusDir, { recursive: true });

  let count = 0;
  for (const topic of TOPICS) {
    for (let part = 1; part <= 6; part += 1) {
      const title = `${topic.title} - Part ${part}`;
      const content = [
        "---",
        `title: "${title}"`,
        "type: resource",
        `tags: [${topic.tag}]`,
        "---",
        "",
        `# ${title}`,
        "",
        topic.sentences.join(" "),
        "",
      ].join("\n");

      fs.writeFileSync(path.join(corpusDir, `${title}.md`), content, "utf8");
      count += 1;
    }
  }

  console.log(`Generated corpus at ${corpusDir} (${count} files)`);
}

export function resolveCorpusDir(corpusDir = CORPUS_DIR) {
  if (!hasMarkdownFiles(corpusDir)) {
    generateCorpus(corpusDir);
  }

  return corpusDir;
}

if (process.argv[1] && path.resolve(process.argv[1]) === __filename) {
  const requestedCorpusDir = process.argv[2] || CORPUS_DIR;
  const existingCorpus = hasMarkdownFiles(requestedCorpusDir);
  const corpusDir = resolveCorpusDir(requestedCorpusDir);
  if (existingCorpus) {
    console.log(`Using DAB corpus at ${corpusDir}`);
  }
}
