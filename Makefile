.PHONY: bench bench-no-build bench-setup

# Fast feedback loop: build + 20 queries (~30s on M1/M3)
bench:
	./scripts/mini-bench.sh

# Skip build, just re-run queries (~1s)
bench-no-build:
	./scripts/mini-bench.sh --no-build

# One-time corpus indexing (run once per machine)
bench-setup:
	python3 scripts/mini-bench-setup.py
