#!/bin/sh
# Installs heavy-cmd's newest release on Linux as $HEAVY_CMD_BIN_DIR/heavy-cmd (~/.local/bin); run it
# again to update. It needs curl. On Windows: install.ps1.
#
#   curl -fsSL https://raw.githubusercontent.com/Ploppy3/heavy-cmd/main/install.sh | sh
set -eu

BIN_DIR="${HEAVY_CMD_BIN_DIR:-$HOME/.local/bin}"
URL=https://github.com/Ploppy3/heavy-cmd/releases/latest/download/heavy-cmd-linux-x86_64

die() {
  printf 'heavy-cmd install: %s\n' "$*" >&2
  exit 1
}

case "$(uname -s) $(uname -m)" in
  "Linux x86_64" | "Linux amd64") ;;
  *) die "no release for $(uname -s) $(uname -m): heavy-cmd is built for Linux and Windows on x86_64" ;;
esac
command -v curl >/dev/null 2>&1 || die "needs curl on PATH"

mkdir -p "$BIN_DIR"
new=$(mktemp "$BIN_DIR/.heavy-cmd-XXXXXX")
trap 'rm -f "$new"' EXIT
curl -fsSL -o "$new" "$URL" || die "cannot download $URL"
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
