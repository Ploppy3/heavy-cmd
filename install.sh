#!/bin/sh
# Installs heavy-cmd's newest release on Linux, or makes sure it is there: run it again at will.
#
#   curl -fsSL https://raw.githubusercontent.com/Ploppy3/heavy-cmd/main/install.sh | sh
#
# It puts the release's binary in $HEAVY_CMD_INSTALL_DIR (~/.local/share/heavy-cmd) as bin/heavy-cmd once
# it runs, keeping the one before as bin/heavy-cmd.old, and links $HEAVY_CMD_BIN_DIR/heavy-cmd
# (~/.local/bin) to it. It needs curl and tar; `heavy-cmd update` moves it on. On Windows: install.ps1.
set -eu

INSTALL_DIR="${HEAVY_CMD_INSTALL_DIR:-$HOME/.local/share/heavy-cmd}"
BIN_DIR="${HEAVY_CMD_BIN_DIR:-$HOME/.local/bin}"
API="${HEAVY_CMD_RELEASES_API:-https://api.github.com/repos/Ploppy3/heavy-cmd/releases/latest}"

say() { printf 'heavy-cmd install: %s\n' "$*" >&2; }
die() {
  say "$*"
  exit 1
}

for tool in curl tar; do
  command -v "$tool" >/dev/null 2>&1 || die "needs $tool on PATH"
done
case "$(uname -s) $(uname -m)" in
  "Linux x86_64" | "Linux amd64") asset=heavy-cmd-linux-x86_64.tar.gz ;;
  *) die "no release for $(uname -s) $(uname -m): heavy-cmd is built for Linux and Windows on x86_64" ;;
esac

# The newest release's tag and this machine's asset, read from GitHub's API without a JSON tool.
release=$(curl -fsSL -H 'Accept: application/vnd.github+json' -H 'User-Agent: heavy-cmd' "$API") || die "cannot read the newest release from $API"
value() { printf '%s' "$release" | grep -o "\"$1\"[[:space:]]*:[[:space:]]*\"[^\"]*\"" | sed 's/.*"\([^"]*\)"$/\1/'; }
tag=$(value tag_name | head -n 1)
url=$(value browser_download_url | grep "/$asset\$" | head -n 1) || true
case "$tag" in
  v[0-9]*.[0-9]*.[0-9]*) ;;
  *) die "$API names no release tag" ;;
esac
[ -n "$url" ] || die "release $tag has no $asset"

bin="$INSTALL_DIR/bin/heavy-cmd"
runs() { "$1" --version 2>&1; }

mkdir -p "$INSTALL_DIR/bin" "$BIN_DIR"
if [ -x "$bin" ] && [ "$(runs "$bin" || true)" = "heavy-cmd $tag" ]; then
  say "$tag is already installed in $bin"
else
  work=$(mktemp -d "$INSTALL_DIR/.install-XXXXXX")
  trap 'rm -rf "$work"' EXIT
  curl -fsSL -o "$work/$asset" "$url" || die "cannot download $url"
  mkdir "$work/heavy-cmd"
  tar -xzf "$work/$asset" -C "$work/heavy-cmd" --strip-components=1 || die "cannot unpack $url"
  said=$(runs "$work/heavy-cmd/bin/heavy-cmd") || die "$tag does not run here: $said"
  [ "$said" = "heavy-cmd $tag" ] || die "$tag does not run here: its --version said \"$said\""
  # The new binary takes the old one's place with one rename; the one before stays as heavy-cmd.old.
  if [ -e "$bin" ]; then
    ln -f "$bin" "$bin.old"
  fi
  mv -f "$work/heavy-cmd/bin/heavy-cmd" "$bin"
fi
: >"$INSTALL_DIR/.heavy-cmd-install"

link="$BIN_DIR/heavy-cmd"
if [ -e "$link" ] && [ ! -L "$link" ]; then
  die "$link exists and is not a link: move it away, then run this again"
fi
ln -sfn "$bin" "$link"

case ":$PATH:" in
  *":$BIN_DIR:"*) ;;
  *) say "$BIN_DIR is not on PATH: add it to run heavy-cmd" ;;
esac
echo "heavy-cmd $tag installed: $link"
