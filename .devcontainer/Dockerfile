# The devcontainer image: the toolchain, the libraries a developer build links,
# the desktop libraries that let the game actually run in here, and podman for
# the release build (which happens in a container of its own -- tools/dist/).
#
# Debian 13 (trixie) is the base because it is the oldest release carrying both
# of the libraries a developer build links as packages: libsdl3-dev, and an
# ffmpeg new enough for rusty_ffmpeg. docs/DEPENDENCIES.md's apt line is this
# list; if the two disagree, that file is the one to fix.
FROM debian:trixie-slim

# Everything docs/DEPENDENCIES.md asks a developer to install, plus what the
# vendored SDL3/ffmpeg builds in tools/build-*.sh need (cmake, ninja, nasm) and
# what the Windows cross-build needs (mingw-w64). docs/WINDOWS.md is the same
# list said another way; if these disagree, that file is the one to fix.
RUN apt-get update && export DEBIAN_FRONTEND=noninteractive \
	&& apt-get install -y --no-install-recommends \
		build-essential \
		pkg-config \
		clang \
		libclang-dev \
		cmake \
		ninja-build \
		nasm \
		git \
		ca-certificates \
		curl \
		python3 \
		file \
		xz-utils \
		zip \
		unzip \
		# The developer build's system libraries.
		libsdl3-dev \
		libavcodec-dev \
		libavformat-dev \
		libavutil-dev \
		libswscale-dev \
		libswresample-dev \
		# rusty_ffmpeg puts libavdevice and libavfilter on the link line
		# whatever the code calls, so a build fails at pkg-config without
		# them -- which is how they were found to be missing from
		# docs/DEPENDENCIES.md's apt line as well.
		libavdevice-dev \
		libavfilter-dev \
		# The Windows cross-toolchain. .cargo/config.toml names
		# x86_64-w64-mingw32-gcc as the linker for x86_64-pc-windows-gnu.
		mingw-w64 \
	&& apt-get clean && rm -rf /var/lib/apt/lists/*

# Fail the image build, not the first cargo build inside it. A base that stopped
# shipping either library is the one failure mode this image has, and it should
# say so here rather than as a pkg-config error forty minutes into someone's
# first build.
RUN pkg-config --exists sdl3 && pkg-config --exists libavcodec \
	|| { echo "base image no longer carries libsdl3-dev / libavcodec-dev" >&2; exit 1; }

ENV RUSTUP_HOME=/usr/local/rustup CARGO_HOME=/usr/local/cargo
ENV PATH=/usr/local/cargo/bin:$PATH

# rustup with **no toolchain**. The compiler this project builds with is pinned
# in rust-toolchain.toml, and rustup installs that one itself the first time
# cargo runs in the workspace -- so the pin stays in the one file that already
# owns it rather than being written down a second time here, where the two could
# disagree. post-create.sh does that first run, and RUSTUP_HOME is a named
# volume (docker-compose.yml) so a rebuild does not download it again.
RUN curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs \
	| sh -s -- -y --default-toolchain none --profile minimal --no-modify-path \
	&& chmod -R a+rwX "$RUSTUP_HOME" "$CARGO_HOME"

# ENV PATH is not enough. A login shell -- which is what a VS Code terminal and
# `bash -l` both are -- sources /etc/profile, and Debian's sets PATH outright
# rather than adding to it, so cargo disappears from exactly the shell a person
# types into. This puts it back after that has run.
RUN printf 'export PATH=%s:$PATH\n' "$CARGO_HOME/bin" > /etc/profile.d/10-cargo.sh

# The supply-chain gate. `just audit` has to work in here -- advisories are
# checked on the machine the build happens on, and that machine is now this
# image -- so cargo-deny is installed rather than left to whoever remembers.
#
# Built with a stable toolchain of its own rather than the project's pin: it is
# a tool that reads deny.toml, not something linked into what ships, and tying
# it to the pin would mean a toolchain bump could fail on cargo-deny's sources.
#
# Pinned, and pinned to the version this project's maintainer runs on the host:
# cargo-deny's own output changes between releases (new lint names, new default
# severities), so an unpinned copy in here would pass or fail differently from
# the same deny.toml run outside, and the disagreement would look like a finding.
# Bumping it is a reviewed change, same as any other pin.
ARG CARGO_DENY_VERSION=0.20.2
RUN rustup toolchain install stable --profile minimal \
	&& cargo +stable install --locked cargo-deny --version "${CARGO_DENY_VERSION}" \
	&& chmod -R a+rwX "$CARGO_HOME"

# Running the game in here, rather than only building it. SDL3's own dependencies
# came in with libsdl3-dev; these are the pieces it dlopen()s or that live
# outside the -dev packages: Mesa's drivers (the container renders on the host's
# GPU through the /dev/dri bind), the Vulkan loader, and the audio clients for
# the PipeWire and PulseAudio sockets bound in from the host session.
#
# vulkan-tools and mesa-utils are diagnostics, not dependencies: when the window
# comes up black, `vulkaninfo --summary` and `glxinfo -B` say whether the GPU
# arrived, which is otherwise guesswork.
RUN apt-get update && export DEBIAN_FRONTEND=noninteractive \
	&& apt-get install -y --no-install-recommends \
		mesa-vulkan-drivers libgl1-mesa-dri libglx-mesa0 libegl-mesa0 libgbm1 \
		libvulkan1 vulkan-tools mesa-utils \
		libpulse0 libasound2t64 \
		libdecor-0-0 \
		# Gamepads reach SDL through evdev; /dev/input is bound in and
		# /run/udev with it, so this is the library that reads them.
		libudev1 \
	&& apt-get clean && rm -rf /var/lib/apt/lists/*

# The editor side. gdb for a crash, less/procps/sudo because a shell without
# them is a shell somebody immediately apt-gets into.
RUN apt-get update && export DEBIAN_FRONTEND=noninteractive \
	&& apt-get install -y --no-install-recommends \
		gdb lldb less procps sudo bash-completion openssh-client \
	&& apt-get clean && rm -rf /var/lib/apt/lists/*

# **Ghidra**, because every recovered rule in this project came out of one.
# CLAUDE.md's rule is decompile first, and that is not possible in a container
# that has no decompiler: a session in here would be back to inferring formats
# from the bytes, which is the single largest source of wasted work this project
# has had.
#
# What is NOT here, and cannot be: the Ghidra *project*. It holds
# `SCHOOLDAYS HQ.exe` and `SysMenuSDHQ.dll`, which are the user's game files,
# and this repository ships no game data. The project directory is a volume --
# or a bind to one you already have, see .env.example -- so an import survives a
# rebuild, which matters because analysing the executable takes a long time.
#
# Pinned with the SHA-256 the release itself publishes, so a substituted or
# truncated download fails the image build. Addresses quoted in this repository
# are Ghidra's default names for the retail build at its default image base; a
# different Ghidra version does not move them, but a different *game* build
# would, and then every recovered claim needs re-checking.
ARG GHIDRA_VERSION=12.1.3
ARG GHIDRA_RELEASE=20260817
ARG GHIDRA_SHA256=93a5d11a9ad510622acaaf908c556a7b9b764d338e78a7567f3689bf5081fd54
RUN apt-get update && export DEBIAN_FRONTEND=noninteractive \
	&& apt-get install -y --no-install-recommends openjdk-21-jdk-headless \
	&& apt-get clean && rm -rf /var/lib/apt/lists/* \
	&& curl -fsSL -o /tmp/ghidra.zip \
		"https://github.com/NationalSecurityAgency/ghidra/releases/download/Ghidra_${GHIDRA_VERSION}_build/ghidra_${GHIDRA_VERSION}_PUBLIC_${GHIDRA_RELEASE}.zip" \
	&& echo "${GHIDRA_SHA256}  /tmp/ghidra.zip" | sha256sum -c - \
	&& unzip -q /tmp/ghidra.zip -d /opt \
	&& mv "/opt/ghidra_${GHIDRA_VERSION}_PUBLIC" /opt/ghidra \
	&& rm /tmp/ghidra.zip \
	&& ln -s /opt/ghidra/support/analyzeHeadless /usr/local/bin/analyzeHeadless

# The two names CLAUDE.md's Ghidra section exports by hand, so a session in here
# does not have to. GHIDRA_PROJ is the mount point docker-compose.yml puts the
# project volume (or your own project directory) on.
ENV GHIDRA_INSTALL_DIR=/opt/ghidra
ENV HEADLESS=/opt/ghidra/support/analyzeHeadless
ENV GHIDRA_PROJ=/root/ghidra_projects

# **Podman inside the devcontainer, and no --privileged.**
#
# A release archive is built in the `builder` image, not in this one, so that
# what ships is not quietly linked against something only the devcontainer has.
# That needs a container runtime in here. Podman rather than dockerd: dockerd
# wants CAP_SYS_ADMIN (which is what the docker-in-docker feature's
# --privileged buys) and its rootless script cannot call newuidmap from an
# unprivileged user in a container, while podman as root in here can.
#
# It needs no daemon, so nothing starts it at postStart. tools/container.sh adds
# the two flags each nested invocation does need -- --network=host, because
# podman's bridge wants CAP_NET_ADMIN this container has not got, and
# --pid=host, because a fresh /proc cannot be mounted over this one's masked
# paths.
RUN apt-get update && export DEBIAN_FRONTEND=noninteractive \
	&& apt-get install -y --no-install-recommends \
		podman crun slirp4netns uidmap \
	&& apt-get clean && rm -rf /var/lib/apt/lists/*

# Written after the install: these are the packages' own conffiles, and a file
# already sitting there stops dpkg to ask which version to keep.
#
# **vfs, not overlay.** fuse-overlayfs works for the image store until a build
# step takes a lock inside it, where it fails with "Unable to lock database".
# vfs copies each layer instead of stacking them, which costs disk and nothing
# else. /var/lib/containers is a named volume (see docker-compose.yml) so a
# built image survives a devcontainer rebuild.
RUN printf '%s\n' \
		'[storage]' \
		'driver = "vfs"' \
		'runroot = "/run/containers/storage"' \
		'graphroot = "/var/lib/containers/storage"' \
		> /etc/containers/storage.conf \
	&& mkdir -p /etc/containers/registries.conf.d \
	&& printf 'unqualified-search-registries = ["docker.io"]\n' \
		> /etc/containers/registries.conf.d/00-docker-io.conf

# The subordinate range podman maps into its containers. 2000:60000 rather than
# the usual 100000:65536, because under a rootless host daemon this container is
# itself given uids 1..65536 -- 100000 does not exist in here and newuidmap
# refuses it. Without a range podman falls back to a single uid and cannot
# unpack an image that owns a file as anything but root.
RUN printf 'root:2000:60000\n' > /etc/subuid \
	&& printf 'root:2000:60000\n' > /etc/subgid

# XDG_RUNTIME_DIR for the container's root user; docker-compose.yml binds the
# host's Wayland, PipeWire and PulseAudio sockets into it. Created here rather
# than left to the runtime so it gets the 0700 the spec requires -- an
# implicitly created mount point is 0755, and libraries that write into the
# runtime directory check the mode.
RUN mkdir -p /run/user/0/pulse && chmod 700 /run/user/0

# Claude Code, from Anthropic's own release channel rather than npm: that
# install method is deprecated and the published binary is self-contained, so
# there is no node in this image at all.
#
# `stable` and `latest` are the moving channels; anything else is taken as a
# version and pinned. The checksum comes from the release's own manifest, so a
# truncated or substituted download fails the image build rather than shipping.
ARG CLAUDE_CODE_VERSION=latest
RUN set -eu; \
	base=https://downloads.claude.ai/claude-code-releases; \
	case "${CLAUDE_CODE_VERSION}" in \
		stable|latest) version=$(curl -fsSL "$base/${CLAUDE_CODE_VERSION}") ;; \
		*) version="${CLAUDE_CODE_VERSION}" ;; \
	esac; \
	case "$(uname -m)" in \
		x86_64) platform=linux-x64 ;; \
		aarch64) platform=linux-arm64 ;; \
		*) echo "no Claude Code build for $(uname -m)" >&2; exit 1 ;; \
	esac; \
	curl -fsSL "$base/$version/manifest.json" \
		| python3 -c "import json,sys; print(json.load(sys.stdin)['platforms']['$platform']['checksum'])" \
		> /tmp/claude.sha256; \
	curl -fsSL -o /usr/local/bin/claude "$base/$version/$platform/claude"; \
	echo "$(cat /tmp/claude.sha256)  /usr/local/bin/claude" | sha256sum -c -; \
	chmod +x /usr/local/bin/claude; \
	rm -f /tmp/claude.sha256; \
	claude --version

# The image owns the version, so the updater has nothing to do: rebuild the
# container to move it. Left on, it would write a second copy of the binary under
# the config volume and the two would disagree about which one `claude` is. The
# second variable is the matching half -- it stops the CLI warning about a binary
# it did not install itself.
ENV DISABLE_AUTOUPDATER=1
ENV DISABLE_INSTALLATION_CHECKS=1

# Mount points for the volumes docker-compose.yml puts here: the SSH directory
# you put your own key in, git's global config, and Claude Code's state. Created
# in the image so the volumes inherit these modes -- .ssh in particular is 0700
# or ssh refuses to use what is in it. **Nothing is copied from the host**; each
# of these starts empty and is yours to fill in once, from inside the container.
RUN mkdir -p /root/.ssh /root/.config/git /root/.claude \
	&& chmod 700 /root/.ssh

# Backed by a named volume, so shell history survives a rebuild.
RUN mkdir -p /commandhistory && touch /commandhistory/.bash_history
ENV HISTFILE=/commandhistory/.bash_history

ENV DEVCONTAINER=true
