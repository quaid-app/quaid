import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

export const CORPUS_DIR = "/tmp/quaid-mini-bench-corpus";

const __filename = fileURLToPath(import.meta.url);
export const __dirname = path.dirname(__filename);

const TOPICS = [
  {
    slug: "agent-memory",
    title: "Agent Memory Architecture",
    paragraphs: [
      "Agent Memory Architecture gives AI agents a durable way to remember goals, preferences, decisions, and evidence across multiple sessions. A strong agent-memory design separates compiled truth from raw timeline notes so Agent Memory Architecture can answer current questions without losing provenance.",
      "In this part, Agent Memory Architecture is framed as a retrieval problem over sessions, documents, and corrections. The agent-memory layer should preserve exact source details while promoting stable facts into a concise working memory for later reasoning.",
      "Operational Agent Memory Architecture also needs conflict handling, versioning, and search paths that make old assumptions visible. Agent-memory systems work best when every remembered claim can point back to the event that created it.",
    ],
  },
  {
    slug: "stablecoin-regulation",
    title: "Stablecoin Regulation",
    paragraphs: [
      "Stablecoin Regulation focuses on reserve quality, redemption rights, issuer disclosures, and the boundary between payments and securities law. Stablecoin Regulation becomes more complex when issuers operate across banking, money transmission, and digital asset regimes.",
      "A practical Stablecoin Regulation review tracks reserve attestations, custodial risk, sanctions screening, and consumer protection. Stablecoin Regulation also asks whether the stablecoin issuer can survive market stress while honoring redemptions.",
      "Policy teams use Stablecoin Regulation analysis to compare federal proposals, state licensing, and international rules. The Stablecoin Regulation keyword should stay close to the title because search users often ask for it directly.",
    ],
  },
  {
    slug: "defi-liquidity",
    title: "DeFi Liquidity Protocol",
    paragraphs: [
      "A DeFi Liquidity Protocol coordinates market makers, liquidity pools, fees, incentives, and price curves without a centralized exchange operator. DeFi Liquidity Protocol design affects slippage, impermanent loss, and capital efficiency.",
      "In automated market makers, the DeFi Liquidity Protocol encodes how tokens move between traders and pooled reserves. DeFi liquidity incentives can bootstrap depth, but they can also attract short-term capital that leaves when rewards decline.",
      "Risk teams compare each DeFi Liquidity Protocol by oracle exposure, governance controls, and liquidation behavior. The phrase DeFi Liquidity Protocol appears often because exact keyword recall matters for benchmark search.",
    ],
  },
  {
    slug: "rust-performance",
    title: "Rust Performance",
    paragraphs: [
      "Rust Performance work usually starts with measuring allocations, lock contention, async runtime overhead, and serialization costs. Rust Performance improves when ownership and borrowing are used to avoid unnecessary clones in hot paths.",
      "A Rust Performance review for web services examines request latency, database batching, cache locality, and backpressure. Rust Performance tuning should be driven by profiles rather than guesses about which abstraction is expensive.",
      "Teams documenting Rust Performance often compare Tokio task scheduling, connection pools, and zero-copy parsing. The Rust Performance benchmark topic should match queries about async runtime performance and service optimization.",
    ],
  },
  {
    slug: "vector-embeddings",
    title: "Vector Embeddings",
    paragraphs: [
      "Vector Embeddings represent text as numeric vectors so semantically related documents can be retrieved even when keywords differ. Vector Embeddings are commonly paired with full-text search to improve recall across large markdown collections.",
      "A Vector Embeddings pipeline includes chunking, model selection, normalization, storage, and approximate nearest neighbor search. Vector Embeddings work best when the chunk text preserves enough context for the model to distinguish similar topics.",
      "Search systems use Vector Embeddings for semantic recall, reranking, and clustering. The Vector Embeddings topic should answer questions about efficient search over document collections and embedding search.",
    ],
  },
  {
    slug: "knowledge-graph",
    title: "Knowledge Graph",
    paragraphs: [
      "A Knowledge Graph stores entities, relationships, temporal links, and evidence so documents can be explored through structured paths. Knowledge Graph traversal helps users understand why two concepts are related instead of only returning isolated pages.",
      "In memory systems, a Knowledge Graph can expose contradictions, orphaned nodes, and stale assertions. Knowledge Graph maintenance requires closing old facts, adding new evidence, and preserving relationship history.",
      "Graph-based reasoning uses Knowledge Graph paths to expand retrieval beyond the first matching document. The Knowledge Graph phrase appears repeatedly because queries often ask about traversal, link reasoning, and contradiction detection.",
    ],
  },
  {
    slug: "smart-contract",
    title: "Smart Contract Security",
    paragraphs: [
      "Smart Contract Security covers reentrancy, access control, arithmetic assumptions, oracle manipulation, and upgrade risk. Smart Contract Security audits examine both individual functions and protocol-level economic behavior.",
      "A Smart Contract Security review should include tests, formal invariants, dependency checks, and deployment controls. Smart Contract Security failures can occur even when each contract compiles cleanly and unit tests pass.",
      "Security teams connect Smart Contract Security with privacy, commitments, and cross-protocol interactions. The Smart Contract Security topic should be easy to find for audit-focused keyword searches.",
    ],
  },
  {
    slug: "cross-chain",
    title: "Cross-Chain Bridge",
    paragraphs: [
      "A Cross-Chain Bridge moves assets or messages between blockchains using validators, light clients, liquidity networks, or lock-and-mint mechanisms. Cross-Chain Bridge design determines trust assumptions and failure modes.",
      "Interoperability teams evaluate each Cross-Chain Bridge by finality handling, replay protection, validator concentration, and emergency controls. Cross-chain asset transfers can expose users to risks from both the source and destination chains.",
      "A robust Cross-Chain Bridge needs monitoring, rate limits, and clear incident response. The Cross-Chain Bridge topic appears in titles and filenames so bridge mechanism queries have strong FTS coverage.",
    ],
  },
  {
    slug: "zero-knowledge",
    title: "Zero Knowledge Proof",
    paragraphs: [
      "A Zero Knowledge Proof lets one party prove a statement without revealing the underlying witness. Zero Knowledge Proof systems often rely on commitments, circuits, proving keys, and verification logic.",
      "Developers use a Zero Knowledge Proof to add privacy, compression, or scalable verification to smart contract workflows. Zero Knowledge Proof circuit design must balance expressiveness, proving cost, and verifier efficiency.",
      "Privacy-focused applications combine Zero Knowledge Proof construction with careful data availability and key management. The Zero Knowledge Proof phrase should match queries about cryptographic commitments and proof circuits.",
    ],
  },
  {
    slug: "rag-retrieval",
    title: "Retrieval Augmented Generation",
    paragraphs: [
      "Retrieval Augmented Generation connects language models to external documents so answers can draw on current, grounded context. Retrieval Augmented Generation reduces context pressure by selecting relevant chunks instead of sending every document.",
      "A RAG retrieval system depends on chunking, hybrid search, reranking, citations, and freshness checks. Retrieval Augmented Generation quality improves when the retriever understands both exact terms and semantic paraphrases.",
      "Teams use Retrieval Augmented Generation to compress large context windows into targeted evidence. The rag-retrieval topic should answer questions about context compression, document stores, and grounded generation.",
    ],
  },
];

function markdownFor(topic, part) {
  const title = `${topic.title} - Part ${part}`;
  const selected = topic.paragraphs
    .map((paragraph, index) =>
      `${paragraph} Part ${part} adds scenario ${index + 1} for ${topic.title}, keeping ${topic.slug} visible for search evaluation.`
    )
    .slice(0, part % 2 === 0 ? 2 : 3);

  return `---
title: "${title}"
type: resource
tags: [${topic.slug}]
---

# ${title}

${selected.join("\n\n")}
`;
}

export function generateCorpus(outputDir = CORPUS_DIR) {
  fs.rmSync(outputDir, { recursive: true, force: true });
  fs.mkdirSync(outputDir, { recursive: true });

  for (const topic of TOPICS) {
    for (let part = 1; part <= 6; part += 1) {
      const filename = `${topic.slug}-part-${String(part).padStart(2, "0")}.md`;
      fs.writeFileSync(path.join(outputDir, filename), markdownFor(topic, part), "utf8");
    }
  }

  return outputDir;
}

if (process.argv[1] && path.resolve(process.argv[1]) === __filename) {
  const outputDir = generateCorpus();
  console.log(`Generated 60 markdown files in ${outputDir}`);
}
