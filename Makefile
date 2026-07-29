# -------------------------------------------------------------------
# Configuration
# -------------------------------------------------------------------

VERSION          ?= $(shell perl -ne 'if (/^version\s*=\s*"(.+)"/) { print $$1; exit }' Cargo.toml */Cargo.toml 2>/dev/null)
ifeq ($(strip $(VERSION)),)
$(error VERSION could not be determined; set VERSION explicitly)
endif
IMAGE            ?= data-connection-hub
CONTAINER_ENGINE ?= $(shell command -v podman 2>/dev/null || command -v docker 2>/dev/null)
V                ?=

ifneq ($(V),)
  _NOCAPTURE := -- --nocapture
endif

.PHONY: all build release check clean \
	test test-unit test-integration \
	lint fmt doc audit check-dco \
	require-container-engine \
	container-flight container-rest container-all \
	container-run-flight container-run-rest \
	setup-hooks help

# -------------------------------------------------------------------
# All
# -------------------------------------------------------------------

all: build fmt lint test audit

# -------------------------------------------------------------------
# Build
# -------------------------------------------------------------------

build:
	cargo build --workspace

release:
	cargo build --workspace --release

check:
	cargo check --workspace

clean:
	cargo clean

# -------------------------------------------------------------------
# Container
# -------------------------------------------------------------------

require-container-engine:
ifndef CONTAINER_ENGINE
	$(error No container engine found — install podman or docker)
endif

container-flight: | require-container-engine
	"$(CONTAINER_ENGINE)" build -t "$(IMAGE)-flight:$(VERSION)" -f flight-service/Containerfile .

container-rest: | require-container-engine
	"$(CONTAINER_ENGINE)" build -t "$(IMAGE)-rest:$(VERSION)" -f rest-service/Containerfile .

container-all: container-flight container-rest

container-run-flight: | require-container-engine
	"$(CONTAINER_ENGINE)" run --rm --network=host \
		-v "$(CURDIR)/flight-service/samples/config.toml:/config/config.toml:ro" \
		"$(IMAGE)-flight:$(VERSION)" 2>&1

container-run-rest: | require-container-engine
	"$(CONTAINER_ENGINE)" run --rm --network=host \
		-v "$(CURDIR)/rest-service/samples/config.toml:/config/config.toml:ro" \
		"$(IMAGE)-rest:$(VERSION)" 2>&1

# -------------------------------------------------------------------
# Test
# -------------------------------------------------------------------

test:
	cargo test --workspace $(_NOCAPTURE)

test-unit:
	cargo test -p commons $(_NOCAPTURE)
	cargo test -p postgres-connector $(_NOCAPTURE)
	cargo test -p pg-meta-store $(_NOCAPTURE)
	cargo test -p rest-service $(_NOCAPTURE)

test-integration:
	cargo test -p flight-service $(_NOCAPTURE)

# -------------------------------------------------------------------
# Quality
# -------------------------------------------------------------------

lint:
	cargo clippy --workspace --all-targets -- -D warnings
	cargo fmt --all -- --check

fmt:
	cargo fmt --all

doc:
	RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps --document-private-items

audit:
	cargo audit

check-dco:
	@bash scripts/check-dco.sh

# -------------------------------------------------------------------
# Dev Setup
# -------------------------------------------------------------------

setup-hooks:
	@mkdir -p .hooks
	ln -sf ../../.hooks/pre-commit .git/hooks/pre-commit
	@echo "Git hooks installed."

# -------------------------------------------------------------------
# Help
# -------------------------------------------------------------------

help:
	@echo "Variables:"
	@echo "  V=1                  show test output (--nocapture)"
	@echo ""
	@echo "Top-level:"
	@echo "  all                  build + fmt + lint + test + audit"
	@echo ""
	@echo "Build:"
	@echo "  build                cargo build --workspace"
	@echo "  release              cargo build --workspace --release"
	@echo "  check                cargo check --workspace"
	@echo "  clean                cargo clean"
	@echo ""
	@echo "Test:"
	@echo "  test                 run all tests"
	@echo "  test-unit            unit tests (commons, postgres-connector, rest-service)"
	@echo "  test-integration     integration tests (flight-service)"
	@echo ""
	@echo "Quality:"
	@echo "  lint                 clippy + rustfmt check"
	@echo "  fmt                  format all crates"
	@echo "  doc                  rustdoc with warnings"
	@echo "  audit                cargo audit"
	@echo ""
	@echo "Container:"
	@echo "  container-flight     build flight-service image"
	@echo "  container-rest       build rest-service image"
	@echo "  container-all        build all service images"
	@echo "  container-run-flight run flight-service container (host network)"
	@echo "  container-run-rest   run rest-service container (host network)"
