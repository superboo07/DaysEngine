#!/usr/bin/env bash
# Start Claude Code inside the devcontainer, from a plain terminal.
#
#   .devcontainer/claude.sh              # interactive session in the container
#   .devcontainer/claude.sh -p "..."     # anything after the name is passed to claude
#
# `up` is idempotent: it starts the container if it is not running and reuses it
# if it is. VS Code is not needed for any of this -- the devcontainer CLI reads
# the same devcontainer.json, and runs it with rootless Podman like VS Code does
# (with "dev.containers.dockerPath": "podman").
set -euo pipefail

cd "$(dirname "$0")/.."

CACHE_DIR=${XDG_CACHE_HOME:-$HOME/.cache}/devcontainers-cli

DEVCONTAINER_CLI=${DEVCONTAINER_CLI:-}
if [ -z "$DEVCONTAINER_CLI" ]; then
	if command -v devcontainer >/dev/null 2>&1; then
		DEVCONTAINER_CLI="devcontainer"
	elif command -v npx >/dev/null 2>&1; then
		DEVCONTAINER_CLI="npx --yes @devcontainers/cli"
	elif command -v node >/dev/null 2>&1; then
		# No npm on this machine, but node alone is enough: unpack the published
		# tarball once and run it from the cache.
		if [ ! -f "$CACHE_DIR/package/devcontainer.js" ]; then
			echo "Fetching @devcontainers/cli into $CACHE_DIR ..." >&2
			mkdir -p "$CACHE_DIR"
			python3 - "$CACHE_DIR" <<-'PY'
				import io, json, sys, tarfile, urllib.request
				dest = sys.argv[1]
				meta = json.load(urllib.request.urlopen("https://registry.npmjs.org/@devcontainers/cli"))
				url = meta["versions"][meta["dist-tags"]["latest"]]["dist"]["tarball"]
				blob = urllib.request.urlopen(url).read()
				tarfile.open(fileobj=io.BytesIO(blob)).extractall(dest)
			PY
		fi
		DEVCONTAINER_CLI="node $CACHE_DIR/package/devcontainer.js"
	else
		echo "error: no 'devcontainer', 'npx' or 'node' on PATH." >&2
		echo "       install the CLI with: npm install -g @devcontainers/cli" >&2
		exit 1
	fi
fi

# The container is run by rootless Podman (devcontainer.json says why). The
# flag goes after the subcommand: before `exec` the CLI would take "exec" as
# the program to run.
ENGINE=(--docker-path podman)

# The CLI runs `podman compose`, which prefers docker-compose when it is
# installed -- and that needs Podman's API socket. podman-compose needs none,
# so use it when it is here and nothing else was chosen (initialize.sh says
# what to do otherwise).
if [ -z "${PODMAN_COMPOSE_PROVIDER:-}" ] && command -v podman-compose >/dev/null 2>&1; then
	export PODMAN_COMPOSE_PROVIDER=podman-compose
fi
export PODMAN_COMPOSE_WARNING_LOGS=false

$DEVCONTAINER_CLI up "${ENGINE[@]}" --workspace-folder .
$DEVCONTAINER_CLI exec "${ENGINE[@]}" --workspace-folder . bash .devcontainer/enable-plugins.sh >&2

# What still needs doing and how to work in here (welcome.sh). post-create's
# output never gets this far -- it is in the build log above, and a reused
# container does not run it at all. To stderr so `claude -p` output stays clean.
# When there is a to-do and a person at the terminal, wait: Claude's UI would
# otherwise push it out of view straight away.
status=0
$DEVCONTAINER_CLI exec "${ENGINE[@]}" --workspace-folder . bash .devcontainer/welcome.sh >&2 || status=$?
if [ "$status" = 10 ] && [ -t 0 ] && [ -t 2 ]; then
	read -r -p "Press Enter to start Claude... " _ >&2 || true
fi

exec $DEVCONTAINER_CLI exec "${ENGINE[@]}" --workspace-folder . claude "$@"
