#!/usr/bin/env bash
# The host's installed plugins are mounted read-only (the container config), but
# enabling one is a setting, and this container's settings.json is its own. Each
# installed plugin not mentioned there yet is enabled; one you disabled in here
# stays disabled. Run by post-create.sh and by claude.sh before every session,
# so a plugin installed on the host later is picked up without a rebuild.
set -euo pipefail

python3 - "${CLAUDE_CONFIG_DIR:-$HOME/.claude}" <<'PY'
import json, os, sys

cfg = sys.argv[1]
manifest = os.path.join(cfg, "plugins", "installed_plugins.json")
if not os.path.isfile(manifest):
    sys.exit(0)
with open(manifest) as f:
    installed = json.load(f).get("plugins", {})

settings_path = os.path.join(cfg, "settings.json")
settings = {}
if os.path.isfile(settings_path):
    with open(settings_path) as f:
        settings = json.load(f)
enabled = settings.setdefault("enabledPlugins", {})
added = [p for p in installed if p not in enabled]
for p in added:
    enabled[p] = True
if added:
    with open(settings_path, "w") as f:
        json.dump(settings, f, indent=2)
        f.write("\n")
    print("Plugins enabled: " + ", ".join(added))
PY
