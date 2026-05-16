export type JsonValue =
  | null
  | boolean
  | number
  | string
  | JsonValue[]
  | { [key: string]: JsonValue };

export interface PlaygroundConfig {
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
}

export interface ApiErrorShape {
  error: string;
  details?: unknown;
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

export interface CliRunRequest {
  id: string;
  params?: Record<string, unknown>;
  confirmed?: boolean;
}

export interface CliRunResult {
  id: string;
  args: string[];
  stdout: string;
  stderr: string;
  exitCode: number;
  parsed?: unknown;
}

export interface TreeNode {
  id: string;
  label: string;
  type: "root" | "directory" | "page" | "raw-import" | "file";
  path?: string;
  children?: TreeNode[];
  meta?: Record<string, unknown>;
}

export interface GraphNode {
  id: string;
  label: string;
  type?: string;
  title?: string;
}

export interface GraphEdge {
  source: string;
  target: string;
  label?: string;
}

export interface GraphData {
  nodes: GraphNode[];
  links: GraphEdge[];
}
