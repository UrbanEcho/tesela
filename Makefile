## Tesela development Makefile.
##
## Tesela Rust and native Python development targets.

UV ?= uv
PYTHON ?= $(UV) run python

.DEFAULT_GOAL := help

.PHONY: help
help: ## Show this help.
	@awk 'BEGIN {FS = ":.*?## "} /^[a-zA-Z_-]+:.*?## / {printf "  \033[36m%-20s\033[0m %s\n", $$1, $$2}' $(MAKEFILE_LIST)

.PHONY: fmt
fmt: rust-fmt ## Format all Rust source files.

.PHONY: check
check: rust-check ## Type-check all Rust crates.

.PHONY: lint
lint: rust-clippy ## Run Clippy with warnings denied.

.PHONY: test
test: rust-test python-test ## Run Rust and Python tests.

.PHONY: build
build: rust-build python-build ## Build Rust crates and the Python extension package.

.PHONY: verify
verify: source-size rust-fmt-check rust-clippy rust-test rust-bench-smoke rust-package python-test python-typecheck python-build docs-check ## Run the local pre-push gate.

.PHONY: source-size
source-size: ## Enforce the 500-line source-file limit.
	$(PYTHON) scripts/check_source_size.py

.PHONY: rust-build
rust-build: ## Build all Rust crates.
	cargo build --workspace

.PHONY: rust-check
rust-check: ## Type-check all Rust crates.
	cargo check --workspace

.PHONY: rust-fmt
rust-fmt: ## Format all Rust source files.
	cargo fmt --all

.PHONY: rust-fmt-check
rust-fmt-check: ## Check Rust formatting.
	cargo fmt --all -- --check

.PHONY: rust-clippy
rust-clippy: ## Run Clippy with warnings denied.
	cargo clippy --workspace --all-targets --all-features -- -D warnings

.PHONY: rust-test
rust-test: ## Run all Rust tests.
	cargo test --workspace

.PHONY: rust-doc
rust-doc: ## Build Rust documentation.
	cargo doc --workspace --no-deps

.PHONY: rust-bench
rust-bench: ## Run Criterion microbenchmarks.
	cargo bench --workspace --exclude tesela-py --all-features

.PHONY: rust-bench-smoke
rust-bench-smoke: ## Compile and smoke-run benchmarks once.
	cargo bench --workspace --exclude tesela-py --all-features -- --test

.PHONY: rust-package
rust-package: ## Build all public .crate archives without publishing.
	cargo package --workspace --exclude tesela-py --allow-dirty --no-verify

.PHONY: python-test
python-test: ## Install the extension and run Python tests.
	$(UV) run --group dev pytest python/tests/ -q

.PHONY: python-typecheck
python-typecheck: ## Type-check public Python usage and package exports.
	$(UV) run --group dev pyright
	$(UV) run --group dev pyright --verifytypes tesela --ignoreexternal

.PHONY: python-build
python-build: ## Build Python sdist/wheel.
	$(UV) build

.PHONY: docs-check
docs-check: ## Verify local Markdown links and version consistency.
	$(PYTHON) scripts/check_release.py

.PHONY: clean
clean: ## Remove local build artifacts.
	rm -rf dist target build *.egg-info
