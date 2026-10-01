#!/usr/bin/env bash
# The Linux release artifact, reproduced locally (M10-T5): the exact
# command sequence the reworked `.github/workflows/create-release.yml`
# runs on a tag push. Builds the release profile of `epic-cli` and the
# desktop shell of `epic-gui`, stages them with the repo's README +
# LICENSE, tars them as
# `epicrouter-<version>-x86_64-unknown-linux-gnu.tar.gz`, and prints
# the tarball's SHA-256 (the release's checksum face).
#
# Version single-source: derived from the workspace `rust/Cargo.toml`
# (`[workspace.package] version`), the same source
# `env!("CARGO_PKG_VERSION")` reads — the script can never drift from
# the binary's `--version` face.
#
# Migration guide (T6): the guide SHIPS IN the tarball —
# `docs/migration-guide.md` is staged beside the README (repo-relative
# reference; the doc's GitHub path is `docs/migration-guide.md` on the
# default branch).

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
WORKSPACE="$(cd "$SCRIPT_DIR/.." && pwd)"
REPO_ROOT="$(cd "$WORKSPACE/.." && pwd)"

VERSION="$(sed -n 's/^version = "\(.*\)"$/\1/p' "$WORKSPACE/Cargo.toml" | head -n1)"
if [ -z "$VERSION" ]; then
    echo "error: could not derive the workspace version from $WORKSPACE/Cargo.toml" >&2
    exit 1
fi

NAME="epicrouter-${VERSION}-x86_64-unknown-linux-gnu"
STAGE="$(mktemp -d)"
trap 'rm -rf "$STAGE"' EXIT
mkdir -p "$STAGE/$NAME"
# The tarball lands in the INVOKING directory (a release artifact the
# caller keeps); only the staging tree is temporary.
OUT_DIR="$(pwd)"

cd "$WORKSPACE"
cargo build --release -p epic-cli
# The desktop shell: --jobs 4 (the desktop-feature cargo cap, buglog
# 194 family).
cargo build --release --jobs 4 -p epic-gui --features desktop

cp target/release/epic-cli "$STAGE/$NAME/epic-cli"
cp target/release/epic-gui "$STAGE/$NAME/epic-gui"
cp "$REPO_ROOT/README.md" "$REPO_ROOT/LICENSE" "$STAGE/$NAME/"
# The migration guide rides the artifact (the T5 slot: a dangling
# pointer in the tarball was rejected at charter time — the doc itself
# ships).
cp "$REPO_ROOT/docs/migration-guide.md" "$STAGE/$NAME/migration-guide.md"
# The staged README's guide link is repo-relative for GitHub; the guide
# sits BESIDE the README in the artifact, so rewrite it on the STAGED
# copy only (the repo file keeps its relative link; T6 fix-round Q2).
sed -i 's#(docs/migration-guide.md)#(migration-guide.md)#' "$STAGE/$NAME/README.md"

tar -czf "$OUT_DIR/$NAME.tar.gz" -C "$STAGE" "$NAME"
echo "wrote $OUT_DIR/$NAME.tar.gz (contents: epic-cli, epic-gui, README.md, LICENSE, migration-guide.md)"
sha256sum "$OUT_DIR/$NAME.tar.gz"
