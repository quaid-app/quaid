# gbrain

Node wrapper package for the GigaBrain CLI. The package stays small and downloads the correct
platform binary from GitHub Releases during `postinstall`.

```bash
npm install -g gbrain
gbrain version
gbrain init ~/brain.db
```

The public npm rollout is staged behind the `v0.9.x` shell-installer test cycle. Until that
publish happens, use this package locally with `npm pack` or install GigaBrain via the shell
installer in the main repository docs.
