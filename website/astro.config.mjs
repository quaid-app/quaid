import { defineConfig } from "astro/config";
import mdx from "@astrojs/mdx";
import starlight from "@astrojs/starlight";

const repo = process.env.GITHUB_REPOSITORY?.split("/")?.[1] ?? "gigabrain";
const owner = process.env.GITHUB_REPOSITORY_OWNER ?? "macro88";
const isGitHubActions = process.env.GITHUB_ACTIONS === "true";

export default defineConfig({
  site: isGitHubActions ? `https://${owner}.github.io` : undefined,
  base: isGitHubActions ? `/${repo}` : "/",
  trailingSlash: "always",
  integrations: [
    starlight({
      title: "GigaBrain",
      description:
        "The personal knowledge brain. SQLite + FTS5 + vector embeddings in one file.",
      customCss: ["./src/styles/custom.css"],
      social: [
        { icon: "github", label: "GitHub", href: "https://github.com/macro88/gigabrain" },
      ],
      sidebar: [
        {
          label: "Overview",
          items: [
            "index",
            "guides/why-gigabrain",
            "guides/how-it-works",
          ],
        },
        {
          label: "Getting Started",
          items: [
            "guides/install",
            "guides/quick-start",
            "guides/getting-started",
          ],
        },
        { label: "CLI Reference", items: ["reference/cli"] },
        { label: "MCP Server", items: ["guides/mcp-server"] },
        {
          label: "Architecture",
          items: ["reference/architecture", "reference/spec"],
        },
        {
          label: "Contributing",
          items: ["contributing/contributing", "contributing/roadmap"],
        },
      ],
    }),
    mdx(),
  ],
});
