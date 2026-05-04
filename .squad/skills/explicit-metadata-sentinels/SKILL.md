# Explicit Metadata Sentinels

Use this when a file format has optional structured metadata at the end of user-authored content.

## Rule

Do not infer metadata from an ambiguous trailing shape alone. Give metadata its own explicit sentinel in the canonical format.

## What to do

1. Pick a marker the user-content path will not emit accidentally (for example a distinct code-fence info string such as `json turn-metadata`).
2. Make the renderer always emit that marker for metadata.
3. Make the parser strip metadata only when the explicit marker is present.
4. Add a round-trip test for the canonical metadata form.
5. Add a second test proving ordinary trailing structured content without the marker stays content.

## Why

Shape-only inference breaks as soon as valid user content can end with the same structure. An explicit sentinel keeps the parser deterministic and preserves user-authored trailing blocks.

## Quaid fit

- `src/core/conversation/format.rs` turn metadata fences
- any future vault markdown surface that appends structured machine metadata after human content
