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
	oc-setup-flight oc-setup-rest oc-setup-all \
	oc-build-flight oc-build-rest oc-build-all \
	sdk-install sdk-test sdk-lint sdk-fmt sdk-typecheck sdk-build sdk-all \
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
	"$(CONTAINER_ENGINE)" build -t "$(IMAGE)-flight:$(VERSION)" -f services/flight/Containerfile .

container-rest: | require-container-engine
	"$(CONTAINER_ENGINE)" build -t "$(IMAGE)-rest:$(VERSION)" -f services/rest/Containerfile .

container-all: container-flight container-rest

container-run-flight: | require-container-engine
	"$(CONTAINER_ENGINE)" run --rm --network=host \
		-v "$(CURDIR)/services/flight/samples/config.toml:/config/config.toml:ro" \
		"$(IMAGE)-flight:$(VERSION)" 2>&1

container-run-rest: | require-container-engine
	"$(CONTAINER_ENGINE)" run --rm --network=host \
		-v "$(CURDIR)/services/rest/samples/config.toml:/config/config.toml:ro" \
		"$(IMAGE)-rest:$(VERSION)" 2>&1

# -------------------------------------------------------------------
# OpenShift Builds
# -------------------------------------------------------------------

OC_NAMESPACE          ?= default
OC_EXCLUDE_SERVICES   ?= (^|/)(\.git|target|dc-controller|docs|\.local|\.claude|\.github)(/|$$)
OC_EXCLUDE_CONTROLLER ?= (^|/)(\.git|target|libs|connectors|services|docs|\.local|\.claude|\.github)(/|$$)
export OC_NAMESPACE OC_EXCLUDE_SERVICES OC_EXCLUDE_CONTROLLER

oc-setup-flight:
	oc apply -k .local/openshift-build/flight-service -n "$${OC_NAMESPACE}"

oc-setup-rest:
	oc apply -k .local/openshift-build/rest-service -n "$${OC_NAMESPACE}"

oc-setup-all: oc-setup-flight oc-setup-rest

oc-build-flight:
	oc start-build flight-service-ubi9 --from-dir=. --follow -n "$${OC_NAMESPACE}" \
		--exclude="$${OC_EXCLUDE_SERVICES}"

oc-build-rest:
	oc start-build rest-service-ubi9 --from-dir=. --follow -n "$${OC_NAMESPACE}" \
		--exclude="$${OC_EXCLUDE_SERVICES}"

oc-build-all: oc-build-flight oc-build-rest

# -------------------------------------------------------------------
# Test
# -------------------------------------------------------------------

test:
	cargo test --workspace $(_NOCAPTURE)

test-unit:
	cargo test -p commons $(_NOCAPTURE)
	cargo test -p postgres-connector $(_NOCAPTURE)
	cargo test -p sqlite-connector $(_NOCAPTURE)
	cargo test -p kube-utils $(_NOCAPTURE)
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
	@bash hack/check-dco.sh

# -------------------------------------------------------------------
# Python SDK
# -------------------------------------------------------------------

PYTHON_SDK_DIR := sdk/python

ifdef VIRTUAL_ENV
  SDK_PYTHON       := python3
  SDK_BIN          :=
  SDK_VENV_PREREQ  :=
else
  SDK_PYTHON       := $(PYTHON_SDK_DIR)/.venv/bin/python3
  SDK_BIN          := .venv/bin/
  SDK_VENV_PREREQ  := $(SDK_PYTHON)
endif

$(PYTHON_SDK_DIR)/.venv/bin/python3:
	python3 -m venv $(PYTHON_SDK_DIR)/.venv

sdk-venv: $(SDK_VENV_PREREQ)

sdk-install: sdk-venv
	$(SDK_PYTHON) -m pip install -e "$(PYTHON_SDK_DIR)[dev]"

sdk-test: sdk-venv
	cd $(PYTHON_SDK_DIR) && $(SDK_BIN)pytest tests/ -v --cov=data_connect_hub --cov-report=term-missing --cov-report=html:htmlcov

sdk-lint: sdk-venv
	cd $(PYTHON_SDK_DIR) && $(SDK_BIN)ruff check src/ tests/
	cd $(PYTHON_SDK_DIR) && $(SDK_BIN)ruff format --check src/ tests/

sdk-fmt: sdk-venv
	cd $(PYTHON_SDK_DIR) && $(SDK_BIN)ruff format src/ tests/
	cd $(PYTHON_SDK_DIR) && $(SDK_BIN)ruff check --fix src/ tests/

sdk-typecheck: sdk-venv
	cd $(PYTHON_SDK_DIR) && $(SDK_BIN)mypy src/

sdk-build: sdk-venv
	$(SDK_PYTHON) -m build $(PYTHON_SDK_DIR)

sdk-all: sdk-lint sdk-typecheck sdk-test

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
	@echo "  test-unit            unit tests (commons, connectors, kube-utils, pg-meta-store, rest-service)"
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
	@echo ""
	@echo "OpenShift Builds (OC_NAMESPACE=default):"
	@echo "  oc-setup-flight      apply flight-service BuildConfig overlay"
	@echo "  oc-setup-rest        apply rest-service BuildConfig overlay"
	@echo "  oc-setup-all         apply all BuildConfig overlays"
	@echo "  oc-build-flight      start flight-service build on cluster"
	@echo "  oc-build-rest        start rest-service build on cluster"
	@echo "  oc-build-all         build all services on cluster"
	@echo ""
	@echo "Python SDK:"
	@echo "  sdk-install          install SDK in editable mode with dev deps"
	@echo "  sdk-test             run SDK unit tests with coverage"
	@echo "  sdk-lint             lint and format-check SDK"
	@echo "  sdk-fmt              format SDK code"
	@echo "  sdk-typecheck        run mypy on SDK"
	@echo "  sdk-build            build SDK distribution"
	@echo "  sdk-all              lint + typecheck + test SDK"
