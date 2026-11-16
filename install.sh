#!/bin/sh
# Installs heavy-cmd's newest release on Linux as $HEAVY_CMD_BIN_DIR/heavy-cmd (~/.local/bin), once its
# SHA-256 is the one the release's SHA256SUMS names; it then updates itself. It needs curl and sha256sum.
# On Windows: install.ps1.
#
#   curl -fsSL https://raw.githubusercontent.com/Ploppy3/heavy-cmd/main/install.sh | sh
set -eu

BIN_DIR="${HEAVY_CMD_BIN_DIR:-$HOME/.local/bin}"
RELEASE=https://github.com/Ploppy3/heavy-cmd/releases/latest/download
ASSET=heavy-cmd-linux-x86_64

die() {
  printf 'heavy-cmd install: %s\n' "$*" >&2
  exit 1
}

case "$(uname -s) $(uname -m)" in
  "Linux x86_64" | "Linux amd64") ;;
  *) die "no release for $(uname -s) $(uname -m): heavy-cmd is built for Linux and Windows on x86_64" ;;
esac
command -v curl >/dev/null 2>&1 || die "needs curl on PATH"
command -v sha256sum >/dev/null 2>&1 || die "needs sha256sum on PATH"

mkdir -p "$BIN_DIR"
new=$(mktemp "$BIN_DIR/.heavy-cmd-XXXXXX")
trap 'rm -f "$new"' EXIT
curl -fsSL --proto =https -o "$new" "$RELEASE/$ASSET" || die "cannot download $RELEASE/$ASSET"
sums=$(curl -fsSL --proto =https "$RELEASE/SHA256SUMS") || die "cannot download $RELEASE/SHA256SUMS"
want=$(printf '%s\n' "$sums" | awk -v name="$ASSET" '$2 == name || $2 == "*" name { print tolower($1); exit }')
[ -n "$want" ] || die "the release's SHA256SUMS names no $ASSET"
got=$(sha256sum "$new" | cut -d' ' -f1)
[ "$got" = "$want" ] || die "$ASSET does not have the SHA-256 its SHA256SUMS names: nothing installed"
chmod 755 "$new"
# One rename: a heavy-cmd running meanwhile keeps the one it started with.
mv -f "$new" "$BIN_DIR/heavy-cmd"

case ":$PATH:" in
  *":$BIN_DIR:"*) ;;
  *) printf 'heavy-cmd install: %s is not on PATH: add it to run heavy-cmd\n' "$BIN_DIR" >&2 ;;
esac
# Its usage's first line names its version.
version=$("$BIN_DIR/heavy-cmd" 2>&1 | head -n 1 | cut -d: -f1)
echo "$version installed: $BIN_DIR/heavy-cmd"
