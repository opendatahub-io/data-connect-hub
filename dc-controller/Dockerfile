# Build the manager binary
FROM registry.access.redhat.com/ubi9/go-toolset:1.26 AS builder
ARG TARGETOS
ARG TARGETARCH

USER root
WORKDIR /workspace

COPY dc-controller/go.mod dc-controller/go.sum ./
RUN go mod download

COPY dc-controller/ .

RUN CGO_ENABLED=0 GOOS=${TARGETOS:-linux} GOARCH=${TARGETARCH} go build -a -o manager cmd/main.go

FROM registry.access.redhat.com/ubi9-minimal:latest

LABEL org.opencontainers.image.description="Data Connect Hub — Kubernetes Controller"

WORKDIR /
COPY --from=builder --chown=root:root --chmod=0555 /workspace/manager .
COPY config/ /manifests/

USER 1001:1001

ENTRYPOINT ["/manager"]
