# Housekeeping / release sequencing

This is an operational batch, not a product-design change. The important design choice is sequencing:

```text
origin/main @ ded7d22
        |
        +--> docs truth repair
        |
        +--> issue reconciliation
        |
        +--> merged-only branch pruning
        |
        '--> release/v0.22.3
                 |
                 +--> Cargo.toml = 0.22.3
                 +--> cargo test
                 '--> tag v0.22.3
```

## Key rules

1. **Roadmap truth precedes tag push.** The public roadmap is currently the stalest public artifact. Repair it before cutting `v0.22.3`.
2. **Published state and branch state stay separate.** Until the tag exists, public install/download copy must keep saying `v0.22.2` is the latest published release.
3. **Branch cleanup is ancestry-gated.** Only delete remote branches already fully merged into `origin/main`. Any branch ahead of main gets an owner review, not an automatic delete.
4. **Release from current main.** The bug fix to ship is already on `origin/main`; the correct release lane starts there.
