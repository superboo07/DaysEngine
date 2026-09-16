#!/usr/bin/env bash
# Start Claude Code inside the devcontainer, from a plain terminal.
#
#   .devcontainer/claude.sh              # interactive session in the container
#   .devcontainer/claude.sh -p "..."     # anything after the name is passed to claude
#
# `up` is idempotent: it starts the container if it is not running and reuses it
# if it is. VS Code is not needed for any of this -- the devcontainer CLI reads
# the same devcontainer.json and docker-compose.yml.
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

$DEVCONTAINER_CLI up --workspace-folder .
exec $DEVCONTAINER_CLI exec --workspace-folder . claude "$@"
