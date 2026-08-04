#!/usr/bin/env bash
# Generates the local, git-ignored credential files that kustomize's
# secretGenerator reads to build the postgres-credentials Secret.
# Run this once before the first `oc apply -k config/db/postgres`.
set -euo pipefail
umask 077

cd "$(dirname "${BASH_SOURCE[0]}")"

ENV_FILE="postgres-credentials.env"
TOML_FILE="secret-config.toml"

if [[ -f "$ENV_FILE" || -f "$TOML_FILE" ]]; then
  echo "Refusing to overwrite existing $ENV_FILE / $TOML_FILE. Remove them first if you want to regenerate." >&2
  exit 1
fi

# The username must stay stable across regenerations: this image's startup
# script only runs ALTER ROLE <user> PASSWORD ... for whatever username is in
# the secret, and can't create a new role on an already-initialized data
# volume. Only the password is safe to randomize per-deploy.
user="dch_user"
password="$(openssl rand -base64 24 | tr -dc 'A-Za-z0-9')"
database="dch_db"

cat > "$ENV_FILE" <<EOF
POSTGRESQL_USER=${user}
POSTGRESQL_PASSWORD=${password}
POSTGRESQL_DATABASE=${database}
EOF

cat > "$TOML_FILE" <<EOF
[database]
url = "postgresql://${user}:${password}@postgres:5432/${database}"
EOF

echo "Generated $ENV_FILE and $TOML_FILE"
