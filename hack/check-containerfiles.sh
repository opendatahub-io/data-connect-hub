#!/usr/bin/env bash
# Verify every workspace member in Cargo.toml is referenced in all Containerfiles.
# This prevents container build failures caused by adding a crate to the workspace
# without updating the Containerfiles.

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "$0")/.." && pwd)"

# --- Parse workspace members from Cargo.toml ---

members=()
in_members=false
while IFS= read -r line; do
  if [[ "$line" =~ ^[[:space:]]*members[[:space:]]*=[[:space:]]*\[ ]]; then
    in_members=true
    continue
  fi
  if $in_members; then
    if [[ "$line" =~ \] ]]; then
      break
    fi
    member=$(echo "$line" | sed -n 's/.*"\([^"]*\)".*/\1/p')
    if [[ -n "$member" ]]; then
      members+=("$member")
    fi
  fi
done < "$REPO_ROOT/Cargo.toml"

if [[ ${#members[@]} -eq 0 ]]; then
  echo "ERROR: Could not parse any workspace members from Cargo.toml"
  exit 1
fi

echo "Workspace members (${#members[@]}): ${members[*]}"

# --- Containerfile paths ---

CONTAINERFILES=(
  "services/flight-service/Containerfile"
  "services/rest-service/Containerfile"
)

errors=0

for cf in "${CONTAINERFILES[@]}"; do
  cf_path="$REPO_ROOT/$cf"
  if [[ ! -f "$cf_path" ]]; then
    echo "ERROR: Containerfile not found: $cf"
    errors=$((errors + 1))
    continue
  fi

  for member in "${members[@]}"; do
    if ! grep -qE "^COPY[[:space:]]+${member}/Cargo\.toml" "$cf_path"; then
      echo "ERROR: $cf: missing 'COPY ${member}/Cargo.toml'"
      errors=$((errors + 1))
    fi

    if ! grep -qE "mkdir -p.*${member}/src" "$cf_path"; then
      echo "ERROR: $cf: missing '${member}/src' in mkdir -p stub creation"
      errors=$((errors + 1))
    fi
  done
done

if [[ $errors -gt 0 ]]; then
  echo ""
  echo "Found $errors error(s). Every workspace member in Cargo.toml must appear in"
  echo "each Containerfile with a COPY <member>/Cargo.toml line and a <member>/src"
  echo "entry in the mkdir -p stub creation command."
  exit 1
fi

echo "All Containerfiles are consistent with workspace members."
