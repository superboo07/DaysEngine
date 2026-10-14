# The devcontainer

A devcontainer holding the toolchain, the libraries a developer build links,
and enough of the desktop session that the game runs in it. It is the
recommended way to work on this project and is not required to build it: the
release archives are built by `tools/in-container.sh`, which needs podman or
docker and nothing else.

It is run by **rootless Podman**, as root inside — which rootless Podman maps to
your own host user, so everything it writes into the tree or an install is owned
by you. The host needs podman and a compose provider (podman-compose, or
docker-compose with `podman.socket` running), and crun as Podman's OCI
runtime for gamepads to open; nothing else.

Open it from a terminal with `.devcontainer/claude.sh` (which brings the
container up, shows what still needs doing, and starts Claude Code inside it),
or in VS Code with **Reopen in Container** after setting the user setting
`"dev.containers.dockerPath": "podman"`. Every new terminal in it shows
`.devcontainer/welcome.sh`: the to-do items (no install mounted, no git
identity, vendored libraries not built) and the commands.

## Your install has to be mounted, and you say where

This project ships no game data. The archive key comes from your
`SCHOOLDAYS HQ.exe` and the UI widget tables from your `SysMenuSDHQ.dll`, so a
container with nothing mounted can build the engine and cannot verify one line
of it.

Copy `.env.example` to `.env` — at the **top of the repository**, not inside
`.devcontainer/` — and fill in the paths:

```bash
cp .env.example .env
```

| Variable | What it is |
| --- | --- |
| `DAYS_GAME_DIR` | the School Days HQ install |
| `DAYS_SHINYDAYS_DIR` | the Shiny Days install, if you have one |
| `DAYS_MOUNT_MODE` | `rw` (default) or `ro`, for both mounts |

**Each install is mounted at its own path.** `/home/you/Games/School Days HQ` on
your machine is that same path inside the container, rather than something like
`/game/schooldays` that only means anything in here. So one value in `.env` is
correct on both sides and nothing has to be translated between them —
`tools/in-container.sh` mounts the source tree the same way, and for the same
reason.

That is what lets three different things read one file:

| Reader | What it takes from it |
| --- | --- |
| `.devcontainer/docker-compose.yml` | what to mount, and the two variables to set inside the container |
| `.vscode/launch.json` | what to pass `--game` for each title |
| you, in a shell | `set -a; . ./.env; set +a` |

`.env` is gitignored — it holds paths from your own machine. A variable left
unset falls back to an empty directory, so the container still comes up and
`daysengine` reports no install rather than the runtime creating an empty
directory where the game was meant to be.

Compose looks for `.env` beside the compose file, so `devcontainer.json`'s
`initializeCommand` (`.devcontainer/initialize.sh`) links the real one into
`.devcontainer/` on the host before the container is built. The same script
builds the image with a plain `podman build`: the Dev Containers CLI builds
through a Dockerfile it generates in `/tmp`, outside the build context, whenever
the service has a `build:` section, so the service names a prebuilt
`localhost/daysengine-devcontainer:latest` instead and the tooling never builds.
Outside VS Code, run that script before `podman compose up`. There is still exactly one `.env`, and both paths are
gitignored.

`DAYS_GAME_DIR` and `DAYS_SHINYDAYS_DIR` are also set *inside* the container, by
`docker-compose.yml` out of that same `.env`. `daysengine --game` reads the first
of them (`src/inspect.rs`), so the inspection subcommands and the tasks in
`.vscode/tasks.json` need no path typed in.

### Playing either title

**Tasks — `run: School Days HQ` and `run: Shiny Days`** — work everywhere with
nothing set up, in here or on a bare host. A task is a shell, so it sources
`.env` itself.

**Launch configurations** — `Play: School Days HQ`, `Play: Shiny Days`, and a
`(release)` each — are the same thing under the debugger, and they are automatic
only in the container. VS Code resolves `${env:...}` in a launch configuration
from its **own** environment, before any shell exists; `envFile` reaches the
debugged process but not the configuration that spawns it. In here, compose put
both variables in the container's environment, so they resolve. On a bare host,
export them first:

```bash
set -a; . ./.env; set +a; code .
```

Without that, `--game` arrives empty and `daysengine` says it found no install,
which is a clear failure rather than a wrong one.

`daysengine` with no `--game` also reads `DAYS_GAME_DIR` now, the way the
inspection subcommands always have, so a shell that has sourced `.env` can just
run it.

Playing writes saves into the install, which is why `rw` is the default. To
verify against something that cannot be touched, point `DAYS_GAME_DIR` at a copy
and set `DAYS_MOUNT_MODE=ro`.

## Running the game in it

`docker-compose.yml` binds in what a window needs: the Wayland socket,
PipeWire and PulseAudio, `/dev/dri` for the host's GPU, and `/dev/input` with
`/run/udev` for gamepads. The host's X server is **not** bound: reaching it needs
your xauth cookie, a host secret, and SDL3 drives Wayland itself.

Gamepads need the last of those and not the first: a pad reaches SDL through
evdev, not over the Wayland socket, which is why a keyboard works in a container
and a controller does not without the bind. `/dev/input` is bound as a directory
so a pad plugged in later appears. Rootless Podman applies no device cgroup, so
what decides whether a node opens is its file permissions — group `input` — and
`group_add: keep-groups` carries your host groups into the container for that.
**That needs crun as the host's OCI runtime**; under runc it is silently a
no-op and no pad opens. Every input node is readable in here, keyboards
included, which the Docker-era cgroup rule also allowed.

When the window comes up black, `vulkaninfo --summary` and `glxinfo -B` say
whether the GPU arrived; both are in the image for that.

## git, SSH and Claude Code are yours, and nothing is copied in

`/root/.ssh`, git's global config and `/root/.claude` are **named volumes that
start empty**. No container socket, ssh agent or session bus reaches in either,
and `devcontainer.json` blanks what VS Code would otherwise forward. No key, config or credential is bound in from the host. Set them
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
again. An HTTP(S) push login is kept in the gitconfig volume too
(`/root/.config/git/credentials`), so it is asked for once.

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
"$HEADLESS" "$GHIDRA_PROJ" SDHQ -import "$DAYS_GAME_DIR/SCHOOLDAYS HQ.exe"
"$HEADLESS" "$GHIDRA_PROJ" SDHQ -import "$DAYS_GAME_DIR/SysMenuSDHQ.dll"
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

Podman inside the devcontainer runs without `--privileged` and without any
container socket from the host. The devcontainer is itself rootless Podman, so
the capabilities it is given apply only inside your own user namespace:

- `SYS_ADMIN` and `seccomp=unconfined` on the devcontainer, for nested podman's
  mounts and namespaces and crun's `pivot_root`.
- `netns = "host"` and `cgroups = "disabled"` in the image's
  `/etc/containers/containers.conf`, because podman's own bridge cannot be made
  in here and the outer `/sys/fs/cgroup` is read-only. `tools/in-container.sh`
  passes `--network=host --cgroups=disabled` to the same end.
- `/var/lib/containers` on a volume, so the image store is native overlay
  rather than overlay stacked on the container's own root, and survives a
  rebuild. A store carried over from the Docker-era container is still vfs;
  `podman system reset` in here empties it and the next build starts overlay.

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

Your own Claude skills and installed plugins are the one exception:
`~/.claude/skills` and `~/.claude/plugins` are bound in **read-only**, over the
volume, so what you have on the host is available in the container and nothing
in the container can change it. Editing a skill or installing a plugin on the
host takes effect without a rebuild (`.devcontainer/enable-plugins.sh` enables
new plugins before every `claude.sh` session); a plugin's own state is a
container volume.
