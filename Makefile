.DEFAULT_GOAL := all

.PHONY: all release test build build-release lint format clean distclean

# Default target
all: build

# Format source code using cargo
format:
	cargo fmt

# Lint source code using cargo
lint:
	cargo clippy --workspace --all-targets -- -D warnings

# Build in debug mode using cargo
build:
	cargo build --workspace

# Build in release mode using cargo
build-release:
	cargo build --workspace --release

# Run all tests using cargo
test:
	cargo test --workspace

# Remove all derivative assets using cargo
distclean:
	cargo clean

# Remove all except binary executable
clean:
	@set -e; \
	TEMP_DIR=$$(mktemp -d); \
	if [ -f target/release/xes ]; then \
		cp target/release/xes "$$TEMP_DIR/xes_release"; \
	fi; \
	if [ -f target/debug/xes ]; then \
		cp target/debug/xes "$$TEMP_DIR/xes_debug"; \
	fi; \
	cargo clean; \
	if [ -f "$$TEMP_DIR/xes_release" ]; then \
		mkdir -p target/release; \
		mv "$$TEMP_DIR/xes_release" target/release/xes; \
	fi; \
	if [ -f "$$TEMP_DIR/xes_debug" ]; then \
		mkdir -p target/debug; \
		mv "$$TEMP_DIR/xes_debug" target/debug/xes; \
	fi; \
	rm -rf "$$TEMP_DIR"

# release: distclean => lint and format code => build in debug mode => test => build in release mode => clean
release: distclean
	$(MAKE) format
	$(MAKE) lint
	$(MAKE) build
	$(MAKE) test
	$(MAKE) build-release
	$(MAKE) clean
