# quaid

Node wrapper package for the Quaid CLI. The package stays small and downloads the `online`
BGE-small release asset from GitHub Releases during `postinstall`.

```bash
npm install -g quaid
quaid version
quaid init ~/.quaid/memory.db
```

The public npm rollout is staged behind the `v0.9.x` release validation cycle. Until that publish
happens, use this package locally with `npm pack` or install Quaid via the shell installer in
the main repository docs.

npm always installs the `online` channel. If you need the offline-safe embedded build, use the
default shell installer or download an `*-airgapped` asset from GitHub Releases.
