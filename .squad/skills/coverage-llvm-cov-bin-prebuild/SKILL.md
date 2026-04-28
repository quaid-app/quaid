# Coverage LLVM-Cov Bin Prebuild

Use this when `cargo llvm-cov` on Windows runs the lib tests but then dies trying to execute a missing `src\main.rs` test binary from `target\llvm-cov-target\debug\deps\quaid-*.exe`.

## Pattern

1. Run the normal coverage command once to confirm the failure mode.
2. Prebuild the bin-test artifact into the llvm-cov target dir with coverage flags:

   ```powershell
   $env:RUSTFLAGS='-C instrument-coverage --cfg=coverage'
   cargo test --no-run --bin quaid --target-dir D:\repos\quaid\target\llvm-cov-target -j 1
   ```

3. Re-run coverage without cleaning:

   ```powershell
   cargo llvm-cov --lib --tests --summary-only --no-clean -j 1
   ```

4. If plain `cargo test` or `cargo llvm-cov` hits linker/file-lock flakiness on Windows, serialize with `-j 1`.

## Why

On this repo, Windows coverage can compile and run `src\lib.rs` tests successfully but still fail because the bin-test executable for `src\main.rs` is absent when llvm-cov tries to launch it. Prebuilding that artifact and reusing the target dir is the cleanest truthful workaround.

## Quaid fit

- `src\main.rs` has unit tests, so the bin-test artifact is real and must exist for full coverage runs.
- This showed up while measuring Batch 1 collection coverage on v0.10.0 work.
