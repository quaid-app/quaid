import fs from "node:fs";
import Database from "better-sqlite3";
import { getConfig } from "./config";
import { ApiError } from "./http";
import type { GraphData, TreeNode } from "./types";

type SqliteDb = InstanceType<typeof Database>;

function openReadonly(): SqliteDb {
  const config = getConfig();
  if (!fs.existsSync(config.dbPath)) {
    throw new ApiError(404, `Quaid DB does not exist: ${config.dbPath}`);
  }
  const db = new Database(config.dbPath, {
    readonly: true,
    fileMustExist: true
  });
  db.pragma("query_only = ON");
  return db;
}

function one<T>(db: SqliteDb, sql: string, params: unknown[] = []): T | undefined {
  return db.prepare(sql).get(...params) as T | undefined;
}

function all<T>(db: SqliteDb, sql: string, params: unknown[] = []): T[] {
  return db.prepare(sql).all(...params) as T[];
}

export function getDbStatus() {
  const config = getConfig();
  if (!fs.existsSync(config.dbPath)) {
    return {
      databaseExists: false,
      dbPath: config.dbPath,
      pageCount: 0,
      collections: [],
      config: {}
    };
  }

  const db = openReadonly();
  try {
    const pageCount = one<{ count: number }>(db, "SELECT COUNT(*) AS count FROM pages")?.count ?? 0;
    const linkCount = one<{ count: number }>(db, "SELECT COUNT(*) AS count FROM links")?.count ?? 0;
    const embeddingJobs = one<{ count: number }>(
      db,
      "SELECT COUNT(*) AS count FROM embedding_jobs WHERE job_state IN ('pending', 'running')"
    )?.count ?? 0;
    const extractionQueue = one<{ count: number }>(
      db,
      "SELECT COUNT(*) AS count FROM extraction_queue WHERE status IN ('pending', 'running')"
    )?.count ?? 0;
    const activeEmbedding = one<{ name: string; dimensions: number }>(
      db,
      "SELECT name, dimensions FROM embedding_models WHERE active = 1 LIMIT 1"
    );
    const configRows = all<{ key: string; value: string }>(
      db,
      "SELECT key, value FROM config WHERE key IN ('embedding_model', 'embedding_dimensions', 'extraction.enabled', 'extraction.model_alias', 'graph_depth') ORDER BY key"
    );
    const collections = getCollectionsFromDb(db);

    return {
      databaseExists: true,
      dbPath: config.dbPath,
      pageCount,
      linkCount,
      embeddingJobs,
      extractionQueue,
      activeEmbedding,
      collections,
      config: Object.fromEntries(configRows.map((row) => [row.key, row.value]))
    };
  } finally {
    db.close();
  }
}

export function getCollections() {
  const db = openReadonly();
  try {
    return getCollectionsFromDb(db);
  } finally {
    db.close();
  }
}

function getCollectionsFromDb(db: SqliteDb) {
  return all<{
    id: number;
    name: string;
    root_path: string;
    state: string;
    writable: number;
    is_write_target: number;
    needs_full_sync: number;
    last_sync_at: string | null;
    page_count: number;
  }>(
    db,
    `SELECT c.id,
            c.name,
            c.root_path,
            c.state,
            c.writable,
            c.is_write_target,
            c.needs_full_sync,
            c.last_sync_at,
            COUNT(p.id) AS page_count
       FROM collections c
       LEFT JOIN pages p ON p.collection_id = c.id
      GROUP BY c.id
      ORDER BY c.is_write_target DESC, c.name ASC`
  );
}

export function getDbTree(): TreeNode[] {
  const db = openReadonly();
  try {
    const pages = all<{
      id: number;
      collection_name: string;
      slug: string;
      type: string;
      title: string;
      updated_at: string;
    }>(
      db,
      `SELECT p.id, c.name AS collection_name, p.slug, p.type, p.title, p.updated_at
         FROM pages p
         JOIN collections c ON c.id = p.collection_id
        WHERE p.quarantined_at IS NULL
        ORDER BY c.name, p.slug
        LIMIT 2000`
    );
    const rawImports = all<{
      id: number;
      collection_name: string;
      slug: string;
      file_path: string;
      created_at: string;
      content_hash: string;
    }>(
      db,
      `SELECT r.id, c.name AS collection_name, p.slug, r.file_path, r.created_at, r.content_hash
         FROM raw_imports r
         JOIN pages p ON p.id = r.page_id
         JOIN collections c ON c.id = p.collection_id
        WHERE r.is_active = 1
        ORDER BY c.name, r.file_path
        LIMIT 2000`
    );

    return [
      makePathTree(
        "pages",
        "Pages",
        pages.map((page) => ({
          id: `page:${page.id}`,
          path: `${page.collection_name}/${page.slug}`,
          label: page.slug.split("/").at(-1) ?? page.slug,
          type: "page" as const,
          meta: page
        }))
      ),
      makePathTree(
        "raw",
        "Raw imports",
        rawImports.map((raw) => ({
          id: `raw:${raw.id}`,
          path: `${raw.collection_name}/${raw.file_path || raw.slug}`,
          label: (raw.file_path || raw.slug).split(/[\\/]/).at(-1) ?? raw.slug,
          type: "raw-import" as const,
          meta: raw
        }))
      )
    ];
  } finally {
    db.close();
  }
}

function makePathTree(rootId: string, label: string, leaves: Array<Omit<TreeNode, "children">>): TreeNode {
  const root: TreeNode = { id: rootId, label, type: "root", children: [] };
  for (const leaf of leaves) {
    const parts = (leaf.path ?? leaf.label).split(/[\\/]/).filter(Boolean);
    let cursor = root;
    parts.slice(0, -1).forEach((part, index) => {
      cursor.children ??= [];
      const existing = cursor.children.find((child) => child.type === "directory" && child.label === part);
      if (existing) {
        cursor = existing;
        return;
      }
      const child: TreeNode = {
        id: `${rootId}:${parts.slice(0, index + 1).join("/")}`,
        label: part,
        type: "directory",
        children: []
      };
      cursor.children.push(child);
      cursor = child;
    });
    cursor.children ??= [];
    cursor.children.push({
      ...leaf,
      label: parts.at(-1) ?? leaf.label
    });
  }
  return root;
}

export function getRawImport(id: number) {
  const db = openReadonly();
  try {
    const row = one<{
      id: number;
      slug: string;
      title: string;
      file_path: string;
      raw_bytes: Buffer;
      content_hash: string;
      created_at: string;
    }>(
      db,
      `SELECT r.id, p.slug, p.title, r.file_path, r.raw_bytes, r.content_hash, r.created_at
         FROM raw_imports r
         JOIN pages p ON p.id = r.page_id
        WHERE r.id = ?1`,
      [id]
    );
    if (!row) {
      throw new ApiError(404, `Raw import not found: ${id}`);
    }
    return {
      id: row.id,
      slug: row.slug,
      title: row.title,
      filePath: row.file_path,
      contentHash: row.content_hash,
      createdAt: row.created_at,
      content: row.raw_bytes.toString("utf8")
    };
  } finally {
    db.close();
  }
}

export function getPageBySlug(slug: string) {
  const db = openReadonly();
  try {
    const normalized = slug.includes("::") ? slug.split("::").slice(1).join("::") : slug;
    const row = one<{
      id: number;
      collection_name: string;
      slug: string;
      uuid: string | null;
      type: string;
      title: string;
      summary: string;
      compiled_truth: string;
      timeline: string;
      frontmatter: string;
      wing: string;
      room: string;
      version: number;
      created_at: string;
      updated_at: string;
    }>(
      db,
      `SELECT p.*, c.name AS collection_name
         FROM pages p
         JOIN collections c ON c.id = p.collection_id
        WHERE p.slug = ?1 OR (c.name || '::' || p.slug) = ?2
        ORDER BY p.updated_at DESC
        LIMIT 1`,
      [normalized, slug]
    );
    if (!row) {
      throw new ApiError(404, `Page not found: ${slug}`);
    }
    const markdown = [
      "---",
      `title: ${row.title}`,
      `type: ${row.type}`,
      `slug: ${row.collection_name}::${row.slug}`,
      "---",
      "",
      row.summary ? `# Summary\n\n${row.summary}` : "",
      row.compiled_truth ? `# Compiled Truth\n\n${row.compiled_truth}` : "",
      row.timeline ? `# Timeline\n\n${row.timeline}` : ""
    ]
      .filter(Boolean)
      .join("\n\n");
    return {
      ...row,
      canonicalSlug: `${row.collection_name}::${row.slug}`,
      markdown
    };
  } finally {
    db.close();
  }
}

export function getGraph(scope: "whole" | "focused", slug?: string, depth = 2): GraphData {
  const db = openReadonly();
  try {
    if (scope === "focused" && slug) {
      return getFocusedGraph(db, slug, depth);
    }
    return getWholeGraph(db);
  } finally {
    db.close();
  }
}

function getWholeGraph(db: SqliteDb): GraphData {
  const nodes = all<{ id: number; slug: string; title: string; type: string }>(
    db,
    "SELECT id, slug, title, type FROM pages WHERE quarantined_at IS NULL ORDER BY updated_at DESC LIMIT 800"
  );
  const nodeIds = new Set(nodes.map((node) => node.id));
  const links = all<{ source: number; target: number; relationship: string }>(
    db,
    `SELECT from_page_id AS source, to_page_id AS target, relationship
       FROM links
      WHERE valid_until IS NULL
      LIMIT 2000`
  ).filter((edge) => nodeIds.has(edge.source) && nodeIds.has(edge.target));
  return {
    nodes: nodes.map((node) => ({
      id: String(node.id),
      label: node.slug,
      title: node.title,
      type: node.type
    })),
    links: links.map((edge) => ({
      source: String(edge.source),
      target: String(edge.target),
      label: edge.relationship
    }))
  };
}

function getFocusedGraph(db: SqliteDb, slug: string, depth: number): GraphData {
  const root = one<{ id: number }>(
    db,
    `SELECT p.id
       FROM pages p
       JOIN collections c ON c.id = p.collection_id
      WHERE p.slug = ?1 OR (c.name || '::' || p.slug) = ?2
      LIMIT 1`,
    [slug.includes("::") ? slug.split("::").slice(1).join("::") : slug, slug]
  );
  if (!root) {
    throw new ApiError(404, `Page not found: ${slug}`);
  }
  const seen = new Set<number>([root.id]);
  let frontier = [root.id];
  const maxDepth = Math.max(0, Math.min(depth, 4));
  for (let i = 0; i < maxDepth; i += 1) {
    const next = new Set<number>();
    for (const id of frontier) {
      const rows = all<{ id: number }>(
        db,
        `SELECT to_page_id AS id FROM links WHERE from_page_id = ?1 AND valid_until IS NULL
         UNION
         SELECT from_page_id AS id FROM links WHERE to_page_id = ?1 AND valid_until IS NULL`,
        [id]
      );
      rows.forEach((row) => {
        if (!seen.has(row.id)) {
          next.add(row.id);
          seen.add(row.id);
        }
      });
    }
    frontier = [...next];
    if (frontier.length === 0 || seen.size > 400) {
      break;
    }
  }
  const placeholders = [...seen].map(() => "?").join(",");
  const nodes = all<{ id: number; slug: string; title: string; type: string }>(
    db,
    `SELECT id, slug, title, type FROM pages WHERE id IN (${placeholders})`,
    [...seen]
  );
  const links = all<{ source: number; target: number; relationship: string }>(
    db,
    `SELECT from_page_id AS source, to_page_id AS target, relationship
       FROM links
      WHERE valid_until IS NULL
        AND from_page_id IN (${placeholders})
        AND to_page_id IN (${placeholders})`,
    [...seen, ...seen]
  );
  return {
    nodes: nodes.map((node) => ({
      id: String(node.id),
      label: node.slug,
      title: node.title,
      type: node.type
    })),
    links: links.map((edge) => ({
      source: String(edge.source),
      target: String(edge.target),
      label: edge.relationship
    }))
  };
}
