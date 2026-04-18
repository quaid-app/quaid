# Scruffy — model resolution test coverage

- Added focused unit coverage in `src/core/inference.rs` for the new alias mappings: `medium -> base` and `max -> m3`.
- Added a normalization test covering known full Hugging Face IDs resolving to canonical aliases and dimensions.
- Added a custom-model acceptance test asserting arbitrary `owner/repo` IDs stay `custom` with `embedding_dim = 0`.
- Validation: `cargo test --quiet` passed after the test updates.
