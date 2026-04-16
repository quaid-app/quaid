## ADDED Requirements

### Requirement: Docs site validates on the same release-readiness surfaces it publishes
The docs-site workflow SHALL build the Astro/Starlight site on pull requests and on pushes to `main` for the content sources that define the published docs experience.

#### Scenario: Pull request changes docs-site content
- **WHEN** a pull request changes the published docs-site source files
- **THEN** the docs workflow runs a build validation without deploying the site

#### Scenario: Main branch updates published docs content
- **WHEN** `main` receives a change to the published docs-site source files
- **THEN** the docs workflow builds the site and deploys the updated output after the build succeeds

### Requirement: Docs site navigation surfaces status, install, release, and contribution paths
The docs site SHALL expose clear entry points for current status, supported install channels, release/coverage information, and contributor workflow pages.

#### Scenario: User lands on the docs homepage
- **WHEN** a user opens the docs site homepage or quick-start path
- **THEN** they can navigate directly to current status, install guidance, release/coverage information, and contribution guidance without reading the full spec first

### Requirement: GitHub Pages deployment preserves correct repository-relative links
The docs site SHALL build with repository-aware base-path behavior so internal links and assets resolve correctly on GitHub Pages.

#### Scenario: Site is served from the repository Pages URL
- **WHEN** the deployed docs site is opened from the repository's GitHub Pages path
- **THEN** internal links and assets resolve correctly under that repository-relative base path
