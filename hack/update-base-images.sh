#!/usr/bin/env bash
# Point the UBI base image pins in the Containerfiles at the current tag
# digests.
#
# Usage:
#   hack/update-base-images.sh
#
# Requires skopeo. registry.redhat.io needs `podman login registry.redhat.io`
# beforehand.
#
# Two refs are refreshed:
#   registry.access.redhat.com/ubi9-minimal     in services/*/Containerfile
#   registry.redhat.io/ubi9/ubi-minimal-pqc     in the *.konflux
#                                               RUNTIME_BASE_IMAGE pins
#
# Left alone on purpose: the ubi9/ubi-minimal:9.7 builder stage (that stream
# is frozen, so its digest is already current) and the go-toolset tag, which
# Renovate bumps.

set -euo pipefail

cd "$(git rev-parse --show-toplevel)"

digest() {
	skopeo inspect --override-os linux --override-arch amd64 \
		--format '{{.Digest}}' "docker://$1"
}

# Rewrite a file through a temp copy: BSD sed (macOS) and GNU sed disagree on
# the -i flag, and this avoids branching on the platform.
replace() {
	local file=$1 pattern=$2 replacement=$3
	sed -E "s|${pattern}|${replacement}|g" "$file" >"$file.tmp"
	mv "$file.tmp" "$file"
}

ubi_minimal_digest=$(digest registry.access.redhat.com/ubi9-minimal:latest)
ubi_minimal_pqc_digest=$(digest registry.redhat.io/ubi9/ubi-minimal-pqc:latest)

echo "ubi-minimal:      ${ubi_minimal_digest}"
echo "ubi-minimal-pqc:  ${ubi_minimal_pqc_digest}"

for file in services/rest/Containerfile services/flight/Containerfile; do
	replace "$file" 'ubi9-minimal@sha256:[0-9a-f]{64}' "ubi9-minimal@${ubi_minimal_digest}"
done

for file in services/rest/Containerfile.konflux \
	services/flight/Containerfile.konflux \
	dc-controller/Containerfile.konflux; do
	replace "$file" 'ubi-minimal-pqc@sha256:[0-9a-f]{64}' "ubi-minimal-pqc@${ubi_minimal_pqc_digest}"
done

git diff --stat -- services/rest/Containerfile \
	services/flight/Containerfile \
	services/rest/Containerfile.konflux \
	services/flight/Containerfile.konflux \
	dc-controller/Containerfile.konflux
