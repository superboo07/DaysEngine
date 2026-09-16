# The devcontainer

A VS Code devcontainer holding the toolchain, the libraries a developer build
links, and enough of the desktop session that the game runs in it. It is the
recommended way to work on this project and is not required to build it: the
release archives are built by `tools/in-container.sh`, which needs podman or
docker and nothing else.

Open it with **Reopen in Container**, or from a terminal with
`.devcontainer/claude.sh` (which brings the container up and starts Claude Code
inside it).

## Your install has to be mounted, and you say where

This project ships no game data. The archive key comes from your
`SCHOOLDAYS HQ.exe` and the UI widget tables from your `SysMenuSDHQ.dll`, so a
container with nothing mounted can build the engine and cannot verify one line
of it.

Copy `.devcontainer/.env.example` to `.devcontainer/.env` and fill in the paths:

```bash
cp .devcontainer/.env.example .devcontainer/.env
```

| Variable | What it is | Where it lands |
| --- | --- | --- |
| `DAYS_GAME_DIR` | the School Days HQ install | `/game/schooldays` |
| `DAYS_SHINYDAYS_DIR` | the Shiny Days install, if you have one | `/game/shinydays` |
| `DAYS_MOUNT_MODE` | `rw` (default) or `ro` | both mounts |

`.env` is read by `docker-compose.yml` and is gitignored — it holds paths from
your own machine. A variable left unset falls back to an empty directory, so the
container still comes up and `daysengine` reports no install rather than the
runtime creating an empty directory where the game was meant to be.

`DAYS_GAME_DIR` is also set *inside* the container, to `/game/schooldays`, and
`daysengine --game` reads it (`src/inspect.rs`) — so the inspection subcommands
and the tasks in `.vscode/tasks.json` need no path typed in.

Playing writes saves into the install, which is why `rw` is the default. To
verify against something that cannot be touched, point `DAYS_GAME_DIR` at a copy
and set `DAYS_MOUNT_MODE=ro`.

## Running the game in it

`docker-compose.yml` binds in what a window needs: the Wayland socket (X11 as
the fallback), PipeWire and PulseAudio, `/dev/dri` for the host's GPU, and
`/dev/input` with `/run/udev` for gamepads. SDL3 picks its video driver from
what it can reach.

Gamepads need the last of those and not the first: a pad reaches SDL through
evdev, not over the Wayland socket, which is why a keyboard works in a container
and a controller does not without the bind. `/dev/input` is bound as a directory
so a pad plugged in later appears.

When the window comes up black, `vulkaninfo --summary` and `glxinfo -B` say
whether the GPU arrived; both are in the image for that.

## git, SSH and Claude Code are yours, and nothing is copied in

`/root/.ssh`, git's global config and `/root/.claude` are **named volumes that
start empty**. No key, config or credential is bound in from the host. Set them
up once inside the container and the volumes keep them across rebuilds:

```bash
git config --global user.name  "you"
git config --global user.email "you@example.com"
```

`GIT_CONFIG_GLOBAL` points git at the volume, which is also what makes the copy
of your `~/.gitconfig` that the Dev Containers extension drops in inert — that
copy names a signing key at a host path that does not exist in the container,
and every commit then fails with "Couldn't load public key".

`CLAUDE_CONFIG_DIR` does the same job for Claude Code: credentials, settings and
per-project history live in the volume, so a rebuild does not ask you to log in
again.

## Ghidra

Ghidra 12.1.3 is in the image at `/opt/ghidra`, with `analyzeHeadless` on
`PATH` and the three names CLAUDE.md's Ghidra section exports by hand already
set: `GHIDRA_INSTALL_DIR`, `HEADLESS`, `GHIDRA_PROJ`. It is pinned by version
and by the SHA-256 the release publishes, so a substituted download fails the
image build.

`analyzeHeadless` on `PATH` is a small wrapper, not Ghidra's own script, and the
difference is the whole point: **Ghidra 12 runs a `.py` script only through
PyGhidra**, and `/opt/ghidra/support/analyzeHeadless` starts a JVM with no
interpreter in it, so a `-postScript` there fails with "Ghidra was not started
with PyGhidra. Python is not available". The wrapper launches the same
`AnalyzeHeadless` class with the same arguments and the same heap and thread
caps, through a virtual environment at `/opt/ghidra-venv` that the image
installs PyGhidra into from the wheels Ghidra itself ships — `--no-index`, so
nothing is fetched and the JPype is the one that release was tested against.

The **project** is not in the image and never in the repository: it holds your
`SCHOOLDAYS HQ.exe` and `SysMenuSDHQ.dll`. `/root/ghidra_projects` is a volume,
so an import survives a rebuild — which matters, because analysing the
executable is slow. If you already have a project on the host, set
`DAYS_GHIDRA_PROJECT_DIR` in `.env` and that directory is bound there instead.

Importing, once, from inside the container:

```bash
"$HEADLESS" "$GHIDRA_PROJ" SDHQ -import "/game/schooldays/SCHOOLDAYS HQ.exe"
"$HEADLESS" "$GHIDRA_PROJ" SDHQ -import "/game/schooldays/SysMenuSDHQ.dll"
```

Then run scripts against it with `-noanalysis`, as CLAUDE.md describes. Nothing
in the engine build depends on any of this — it is a research tool, and `cargo
build`, the tests and the `daysengine` subcommands all work without a project.

## Building a release archive

Not in the devcontainer. That image carries Mesa, the Vulkan loader, a debugger,
podman and Claude Code; an archive built there would take its glibc floor from
whatever that image happened to be based on, which is an accident rather than a
decision.

**Nothing new has to be run to get that right.** The three scripts that build
what ships -- `tools/build-sdl.sh`, `tools/build-ffmpeg.sh` and `tools/dist.sh`
-- each source `tools/in-container.sh` at their top, which asks one question
before the script does anything:

| where it is | what happens |
| --- | --- |
| already in the build image | runs, no nesting |
| in the devcontainer | podman, with `--network=host --cgroups=disabled`, re-execs the script in the build image |
| on a bare host | podman or docker, whichever is installed, same re-exec |

So `./tools/dist.sh linux` means the same thing in all three places, the tasks in
`.vscode/tasks.json` and the recipes in the `justfile` call the scripts exactly
as they always did, and someone who never opens the devcontainer runs the
identical command.

The image it builds in is `tools/dist/Dockerfile`: a toolchain, the MinGW
cross-compiler, the assembler, and nothing else, with the pinned SDL3 and ffmpeg
sources from `third_party/` built inside it. It is rebuilt on its own when that
Dockerfile or the toolchain pin changes, because the label it carries is a hash
of both -- "an image by that name exists" is the wrong question, and answering it
costs a whole build when the Dockerfile has moved underneath you.

A developer build goes through the same image for the same libraries: `just deps`
builds the pinned SDL3 and ffmpeg into `target/`, once per checkout, and
`.cargo/config.toml` points cargo at them, so a plain `cargo build` links what a
release links. `docs/DEPENDENCIES.md` has the reasoning, including why this is
no longer the distribution's ffmpeg.

Podman inside the devcontainer runs without `--privileged`. Three things make
that work, and each was measured in this image rather than copied from
elsewhere:

- `--network=host` on the nested run, because podman's own bridge wants
  `CAP_NET_ADMIN` the devcontainer has not got.
- `--cgroups=disabled` on the nested run, because the outer container's
  `/sys/fs/cgroup` is read-only and creating the container's cgroup otherwise
  fails on `cgroup.subtree_control`. Podman then insists on a private PID
  namespace, so `--pid=host` is *not* passed — the two are mutually exclusive.
- `systempaths=unconfined` on the devcontainer itself, because a private PID
  namespace needs a fresh `/proc` and the kernel refuses to mount one while the
  outer `/proc` carries the runtime's masked overmounts.

Who the build runs as is decided by asking the runtime, not by assuming. Under
**rootless** podman or docker, container root already *is* the caller, so the
default is correct and passing `--user` would map into the subuid range and fail
to write `target/` at all -- which is exactly how this was found. Under a root
daemon the build runs as the caller's uid, with `HOME` and `CARGO_HOME` under
`target/`, so the archive comes out owned by the person who asked for it.

## The toolchain pin

`rust-toolchain.toml` is the only place the compiler version is written. The
devcontainer image installs rustup with **no toolchain** and `post-create.sh`
lets rustup act on the file; `RUSTUP_HOME` is a volume, so a rebuild does not
download it again. The release builder image takes the same value as a build
argument, read from the same file by `tools/in-container.sh`.

Your own Claude skills are the one exception: `~/.claude/skills` is bound in
**read-only**, over the volume, so what you wrote on the host is available in
the container and nothing in the container can change it. Editing a skill on the
host takes effect without a rebuild.
