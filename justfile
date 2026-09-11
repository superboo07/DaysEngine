# All recipes pass --locked: a build must never silently rewrite Cargo.lock.
# If a recipe fails with "the lock file needs to be updated", that is the point.
# Update it deliberately with `just add` / `just update`, then review the diff.

default: check

build:
    cargo build --locked --workspace

release:
    cargo build --locked --release --workspace

check:
    cargo fmt --all -- --check
    cargo clippy --locked --workspace --all-targets -- -D warnings
    cargo test --locked --workspace

# Supply-chain gate: advisories, licenses, banned crates, source registries.
audit:
    cargo deny --locked check

# Deliberate dependency change. Review the Cargo.lock diff before committing.
update:
    cargo update --locked --dry-run || true
    @echo "Review the above, then run: cargo update -p <crate> --precise <version>"

# Prove a fresh clone reproduces: no lockfile drift after a full build.
verify-lock: build
    git diff --exit-code Cargo.lock
