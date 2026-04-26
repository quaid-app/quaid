import { defineConfig } from "astro/config";
import mdx from "@astrojs/mdx";
import starlight from "@astrojs/starlight";

const repo = process.env.GITHUB_REPOSITORY?.split("/")?.[1] ?? "quaid";
const owner = process.env.GITHUB_REPOSITORY_OWNER ?? "quaid-app";
const isGitHubActions = process.env.GITHUB_ACTIONS === "true";

export default defineConfig({
  site: isGitHubActions ? `https://${owner}.github.io` : undefined,
  base: isGitHubActions ? `/${repo}` : "/",
  trailingSlash: "always",
  integrations: [
    starlight({
      title: "Quaid",
      description:
        "The personal knowledge brain. SQLite + FTS5 + vector embeddings in one file.",
      customCss: ["./src/styles/custom.css"],
      components: {
        Header: "./src/components/Header.astro",
        PageTitle: "./src/components/PageTitle.astro",
      },
      social: [
        {
          icon: "github",
          label: "GitHub",
          href: "https://github.com/quaid-app/quaid",
        },
      ],
      sidebar: [
        {
          label: "Start here",
          items: ["start-here/welcome", "why-quaid"],
        },
        {
          label: "Tutorials",
          items: [
            "tutorials/install",
            "tutorials/first-brain",
            "tutorials/connect-claude-code",
          ],
        },
        {
          label: "How-to guides",
          items: [
            "how-to/import-obsidian",
            "how-to/airgapped-vs-online",
            "how-to/switch-embedding-model",
            "how-to/write-pages",
            "how-to/build-graph",
            "how-to/contradictions-and-gaps",
            "how-to/collections",
            "how-to/skills",
            "how-to/upgrade",
            "how-to/troubleshooting",
          ],
        },
        {
          label: "Explanation",
          items: [
            "explanation/architecture",
            "explanation/page-model",
            "explanation/hybrid-search",
            "explanation/skills-system",
            "explanation/embedding-models",
            "explanation/privacy",
          ],
        },
        {
          label: "Reference",
          items: [
            "reference/cli",
            "reference/mcp",
            "reference/configuration",
            "reference/schema",
            "reference/page-types",
            "reference/errors",
          ],
        },
        {
          label: "For agents",
          items: [
            "agents/quickstart",
            "agents/tool-catalog",
            "agents/skill-workflows",
            "agents/sensitivity-contract",
          ],
        },
        {
          label: "Integrations",
          items: ["integrations/hermes"],
        },
        {
          label: "Contribute",
          items: [
            "contributing/contributing",
            "contributing/roadmap",
            "contributing/specification",
          ],
        },
      ],
    }),
    mdx(),
  ],
});
