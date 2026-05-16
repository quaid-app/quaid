export interface ApiStatus {
  config: {
    rootDir: string;
    profilesDir: string;
    quaidBin: string;
    dbPath: string;
    embeddingModel: string;
    httpPort: number;
    mcpUrl: string;
    runtimeMode: "managed" | "external";
    cliFallbackForMcp: boolean;
    autoInitDb: boolean;
    autoEnableExtraction: boolean;
  };
  runtime: {
    mode: "managed" | "external";
    mcpUrl: string;
    managedRunning: boolean;
    pid: number | null;
    logTail: string[];
  };
  database: DbStatus;
}

export interface DbStatus {
  databaseExists: boolean;
  dbPath: string;
  pageCount: number;
  linkCount?: number;
  embeddingJobs?: number;
  extractionQueue?: number;
  activeEmbedding?: {
    name: string;
    dimensions: number;
  };
  collections: CollectionInfo[];
  config: Record<string, string>;
}

export interface CollectionInfo {
  id: number;
  name: string;
  root_path: string;
  state: string;
  writable: number;
  is_write_target: number;
  needs_full_sync: number;
  last_sync_at: string | null;
  page_count: number;
}

export interface SearchResult {
  slug: string;
  title?: string;
  summary?: string;
  score?: number;
  wing?: string;
  type?: string;
  [key: string]: unknown;
}

export interface MemoryPage {
  slug: string;
  canonicalSlug?: string;
  title: string;
  type: string;
  summary: string;
  compiled_truth: string;
  timeline: string;
  wing: string;
  room: string;
  version: number;
  updated_at: string;
  markdown?: string;
  [key: string]: unknown;
}

export interface EnrichedResult {
  result: SearchResult;
  page?: MemoryPage;
  error?: string;
}

export interface TreeNode {
  id: string;
  label: string;
  type: "root" | "directory" | "page" | "raw-import" | "file";
  path?: string;
  children?: TreeNode[];
  meta?: Record<string, unknown>;
}

export interface CliCommandSpec {
  id: string;
  label: string;
  description: string;
  risky: boolean;
  params: Array<{
    name: string;
    label: string;
    required?: boolean;
    defaultValue?: string | number | boolean;
    type?: "string" | "number" | "boolean" | "json";
  }>;
}

export interface CliRunResult {
  id: string;
  args: string[];
  stdout: string;
  stderr: string;
  exitCode: number;
  parsed?: unknown;
}

export interface GraphData {
  nodes: Array<{
    id: string;
    label: string;
    type?: string;
    title?: string;
  }>;
  links: Array<{
    source: string;
    target: string;
    label?: string;
  }>;
}

export interface EmbeddingProfile {
  name: string;
  modelAlias: string;
  dbPath: string;
  createdAt: string;
}
