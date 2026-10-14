#!/usr/bin/env bash
# **Runs on the host, before the container exists** (devcontainer.json's
# initializeCommand -- VS Code, the devcontainer CLI and claude.sh alike).
#
# 1. Compose looks for .env beside the compose file; this project keeps it at
#    the top of the tree, where .vscode/launch.json and a plain shell read the
#    same one. Linked rather than copied so there is exactly one and it cannot
#    drift, and only when there is something to link to, so a fresh clone gets
#    no dangling symlink (compose then falls back to /var/empty, the designed
#    answer for "no install"). Both paths are gitignored by the `*.env` rule.
#
# 2. The dev container tooling runs `podman compose`, which hands the work to an
#    external provider: docker-compose if it is installed, else podman-compose.
#    docker-compose talks to Podman through its API socket, which is not running
#    unless enabled, and then `up` fails with "failed to connect to the docker
#    API at unix:///run/user/<uid>/podman/podman.sock". Say so here, with the
#    fix, instead of letting that surface from deep in the tooling's log.
#
# 3. The image is built here, with a plain `podman build` (rootless, as you),
#    and the container config only names it. The dev container tooling never
#    builds it: its generated Dockerfile-with-features build lives outside the
#    build context (Docker's buildx bake refuses it outright), and a plain build
#    of this directory is the same on every engine. Layer caching keeps it
#    quick when nothing changed; "Rebuild Container" runs it again.
#    CLAUDE_CODE_VERSION in the host environment (stable, latest, or a version)
#    is passed through; add --no-cache by hand to re-fetch a moving channel.
set -euo pipefail

cd "$(dirname "$0")/.."

if ! command -v podman >/dev/null 2>&1; then
    echo "error: this dev container is run by rootless Podman, and podman is not on PATH." >&2
    exit 1
fi

if [ -f .env ]; then
    ln -sfn ../.env .devcontainer/.env
fi

provider=$(PODMAN_COMPOSE_WARNING_LOGS=true podman compose version 2>&1 | sed -n 's/.*external compose provider "\([^"]*\)".*/\1/p' | head -n1)
if [ -z "$provider" ]; then
    cat >&2 <<'MSG'
error: `podman compose` found no compose provider. Install podman-compose
       (or docker-compose) on the host -- the one thing besides podman this
       container needs, because it is a compose project.
MSG
    exit 1
fi
case "$(basename "$provider")" in
    docker-compose)
        if ! podman --remote info >/dev/null 2>&1; then
            cat >&2 <<MSG
error: \`podman compose\` uses $provider, which needs Podman's API socket,
       and it is not running. Either start it (and keep it across logins):
           systemctl --user enable --now podman.socket
       or make podman use podman-compose instead, which needs no socket:
           export PODMAN_COMPOSE_PROVIDER=podman-compose
       (claude.sh does the latter by itself when podman-compose is installed).
MSG
            exit 1
        fi
        ;;
esac

# Gamepads open only when the container keeps your host groups (`input`), and
# keep-groups works only under crun; runc accepts it and silently drops it
# (docker-compose.yml). Not fatal -- everything else works -- but said here,
# because inside the container it looks like there are simply no pads.
if [ "$(podman info --format '{{.Host.OCIRuntime.Name}}' 2>/dev/null)" != crun ]; then
    echo "warning: Podman's OCI runtime is not crun, so gamepads will not open in the container." >&2
fi

build_args=()
if [ -n "${CLAUDE_CODE_VERSION:-}" ]; then
    build_args+=(--build-arg "CLAUDE_CODE_VERSION=$CLAUDE_CODE_VERSION")
fi

podman build "${build_args[@]}" -t localhost/daysengine-devcontainer:latest .devcontainer
