# Contributing to Data Connect Hub

Thank you for your interest in contributing to Data Connect Hub! This guide covers everything you need to get started.

## Prerequisites

- **Rust 1.96+** — the project pins the toolchain via [`rust-toolchain.toml`](rust-toolchain.toml)
- **PostgreSQL** — required for integration tests
- **Docker or Podman** — required for container builds

## Getting Started

```bash
git clone git@github.com:opendatahub-io/data-connect-hub.git
cd data-connect-hub
make build
make test-unit
```

## Development Commands

| Command | Description |
|---------|-------------|
| `make build` | Build all workspace crates |
| `make test` | Run all tests (unit + integration) |
| `make test-unit` | Run unit tests only |
| `make test-integration` | Run integration tests (requires PostgreSQL) |
| `make lint` | Run clippy and format check |
| `make fmt` | Auto-format all crates |
| `make doc` | Build rustdoc with warnings as errors |
| `make audit` | Run `cargo audit` for known vulnerabilities |
| `make check-dco` | Verify DCO sign-off on commits locally |
| `make all` | Run build + fmt + lint + test + audit |

Pass `V=1` to any test target for verbose output (e.g. `make test-unit V=1`).

## Code Style

- **Clippy**: all warnings are treated as errors (`-D warnings`)
- **Formatting**: enforced via `cargo fmt --check` in CI — run `make fmt` before committing
- **Documentation**: `rustdoc` runs with `-D warnings`, so broken doc links or missing docs on public items will fail CI
- **Unsafe code**: denied workspace-wide via `unsafe_code = "deny"`

## Commit Requirements

All commits in a pull request must satisfy two requirements:

### 1. DCO Sign-off

Every commit must include a `Signed-off-by:` trailer, certifying that you have the right to submit the code under the project's license ([Developer Certificate of Origin](https://developercertificate.org/)).

```bash
# Sign off a new commit
git commit -s -m "your commit message"

# Amend the last commit to add sign-off
git commit -s --amend --no-edit

# Sign off multiple commits retroactively
git rebase HEAD~N --signoff
```

### 2. Signed Commits (GPG/SSH)

Every commit must have a valid cryptographic signature. GitHub supports both GPG and SSH signing.

**Setting up GPG signing:**

```bash
# List your GPG keys
gpg --list-secret-keys --keyid-format=long

# Configure git to use your key
git config --global user.signingkey <YOUR_KEY_ID>
git config --global commit.gpgsign true
```

**Setting up SSH signing:**

```bash
git config --global gpg.format ssh
git config --global user.signingkey ~/.ssh/id_ed25519.pub
git config --global commit.gpgsign true
```

After configuring either method, add your public key to your [GitHub account settings](https://github.com/settings/keys).

See GitHub's [signing commits documentation](https://docs.github.com/en/authentication/managing-commit-signature-verification/signing-commits) for full details.

### Retroactively Signing and Signing-off Commits

If you have existing commits on a branch that are missing signatures or DCO sign-off:

```bash
# Sign all commits on the branch (since it diverged from main)
git rebase --exec 'git commit --amend --no-edit -S' main

# Add both DCO sign-off and signatures
git rebase --signoff --exec 'git commit --amend --no-edit -S' main

# Force-push after rewriting history (SHAs will change)
git push --force-with-lease
```

## Python SDK Development

A virtual environment at `sdk/python/.venv` is created automatically on first run.
If `VIRTUAL_ENV` is already set (e.g. a manually activated venv), the Makefile uses the system Python directly.

```bash
make sdk-install     # install in editable mode with dev deps
make sdk-test        # run tests with coverage
make sdk-lint        # ruff check + format check
make sdk-fmt         # auto-format
make sdk-typecheck   # run mypy strict type checking
make sdk-all         # lint + typecheck + test
```

## Pull Request Process

1. Fork the repository and create a feature branch from `main`
2. Make your changes, ensuring all commits are signed and have DCO sign-off
3. Run `make all` locally to verify everything passes
4. Open a pull request against `main`

### CI Checks

Every PR must pass the following checks before merge:

| Check | What it verifies |
|-------|-----------------|
| **Build and Test** | `cargo build`, clippy, fmt check, unit tests, rustdoc, `cargo audit` |
| **DCO Sign-off** | All commits have a `Signed-off-by:` trailer |
| **Signed Commits** | All commits have a valid GPG/SSH signature |
