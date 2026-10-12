#!/usr/bin/env bash
# **Runs on the host, before the container exists** -- devcontainer.json's
# initializeCommand, from the top of the tree.
set -euo pipefail

cd "$(dirname "$0")/.."

# Compose looks for .env beside the compose file; this project keeps it at the
# top of the tree, where .vscode/launch.json and a plain shell read the same
# one. Linked rather than copied so there is exactly one and it cannot drift,
# and only when there is something to link to, so a fresh clone gets no
# dangling symlink (compose then falls back to /var/empty, the designed answer
# for "no install"). Both paths are gitignored by the `*.env` rule.
if [ -f .env ]; then
    ln -sfn ../.env .devcontainer/.env
fi

# **The image is built here, not by compose.** When the compose service has a
# `build:` section, the Dev Containers CLI writes a Dockerfile-with-features
# into /tmp and points compose at it. Compose 5 builds through buildx bake,
# and bake refuses to read a Dockerfile outside the build context without an
# fs.read entitlement nobody can grant from this repository -- so the build
# died before it started. Given a plain `image:` and no features, the CLI adds
# its labels through the compose override instead and writes no Dockerfile at
# all. A plain `docker build` with the context inside .devcontainer/ needs no
# entitlement. The layer cache makes this a no-op when nothing changed.
# CLAUDE_CODE_VERSION in the host environment (stable, latest, or a version)
# is passed through to the Dockerfile's ARG of the same name.
build_args=()
if [ -n "${CLAUDE_CODE_VERSION:-}" ]; then
    build_args+=(--build-arg "CLAUDE_CODE_VERSION=$CLAUDE_CODE_VERSION")
fi

docker build "${build_args[@]}" -t daysengine-devcontainer:latest .devcontainer
