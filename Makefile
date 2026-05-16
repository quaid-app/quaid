.PHONY: bench bench-no-build bench-setup build build-debug build-online build-cross

# Build targets
build-debug:
	cargo build

build:
	cargo build --release

build-online:
	cargo build --release --no-default-features --features bundled,online-model

build-cross-darwin-arm64:
	cross build --release --target aarch64-apple-darwin

build-cross-darwin-x64:
	cross build --release --target x86_64-apple-darwin

build-cross-linux-x64:
	cross build --release --target x86_64-unknown-linux-musl

build-cross-linux-arm64:
	cross build --release --target aarch64-unknown-linux-musl

# Benchmark targets
# Fast feedback loop: build + 20 queries (~30s on M1/M3)
bench:
	./scripts/mini-bench.sh

# Skip build, just re-run queries (~1s)
bench-no-build:
	./scripts/mini-bench.sh --no-build

# One-time corpus indexing (run once per machine)
bench-setup:
	node scripts/mini-bench-setup.mjs --quaid ./target/release/quaid
