#!/bin/sh
# Installs heavy-cmd's newest release, or makes sure it is there: run it again at will.
#
#   curl -fsSL https://raw.githubusercontent.com/Ploppy3/heavy-cmd/main/install.sh | sh
#
# It unpacks the release into $HEAVY_CMD_INSTALL_DIR (~/.local/share/heavy-cmd) as versions/<tag>,
# points current at it once it runs, and links $HEAVY_CMD_BIN_DIR/heavy-cmd (~/.local/bin) to
# current/bin/heavy-cmd. It needs Node 26 or later, curl and tar; `heavy-cmd update` moves it on.
set -eu

INSTALL_DIR="${HEAVY_CMD_INSTALL_DIR:-$HOME/.local/share/heavy-cmd}"
BIN_DIR="${HEAVY_CMD_BIN_DIR:-$HOME/.local/bin}"
API="${HEAVY_CMD_RELEASES_API:-https://api.github.com/repos/Ploppy3/heavy-cmd/releases/latest}"

say() { printf 'heavy-cmd install: %s\n' "$*" >&2; }
die() {
  say "$*"
  exit 1
}

for tool in node curl tar; do
  command -v "$tool" >/dev/null 2>&1 || die "needs $tool on PATH"
done
major=$(node -p 'process.versions.node.split(".")[0]')
[ "$major" -ge 26 ] || die "needs Node 26 or later, for node:sqlite; this is $(node --version)"

# The newest release's tag and tarball, as GitHub's API gives them.
release=$(curl -fsSL -H 'Accept: application/vnd.github+json' "$API") || die "cannot read the newest release from $API"
info=$(printf '%s' "$release" | node -e '
  let text = "";
  process.stdin.on("data", (chunk) => (text += chunk)).on("end", () => {
    const release = JSON.parse(text);
    const asset = (release.assets || []).find((each) => each.name === "heavy-cmd.tar.gz");
    if (!/^v\d+\.\d+\.\d+$/.test(release.tag_name || "") || !asset) process.exit(1);
    console.log(`${release.tag_name} ${asset.browser_download_url}`);
  });') || die "$API names no release with a heavy-cmd.tar.gz"
tag=${info%% *}
url=${info#* }

runs() { node --disable-warning=ExperimentalWarning "$1/bin/heavy-cmd" --version 2>&1; }

mkdir -p "$INSTALL_DIR/versions" "$BIN_DIR"
target="$INSTALL_DIR/versions/$tag"
if [ -d "$target" ] && [ "$(runs "$target" || true)" = "heavy-cmd $tag" ]; then
  say "$tag is already installed in $target"
else
  work=$(mktemp -d "$INSTALL_DIR/versions/.$tag-XXXXXX")
  trap 'rm -rf "$work"' EXIT
  curl -fsSL -o "$work/heavy-cmd.tar.gz" "$url" || die "cannot download $url"
  mkdir "$work/heavy-cmd"
  tar -xzf "$work/heavy-cmd.tar.gz" -C "$work/heavy-cmd" --strip-components=1 || die "cannot unpack $url"
  said=$(runs "$work/heavy-cmd") || die "$tag does not run here: $said"
  [ "$said" = "heavy-cmd $tag" ] || die "$tag does not run here: its --version said \"$said\""
  rm -rf "$target"
  mv "$work/heavy-cmd" "$target"
fi

# current switches with one rename; without GNU mv's -T, the old link goes first.
ln -s "versions/$tag" "$INSTALL_DIR/.current-$$"
if ! mv -fT "$INSTALL_DIR/.current-$$" "$INSTALL_DIR/current" 2>/dev/null; then
  rm -f "$INSTALL_DIR/current"
  mv -f "$INSTALL_DIR/.current-$$" "$INSTALL_DIR/current"
fi

link="$BIN_DIR/heavy-cmd"
if [ -e "$link" ] && [ ! -L "$link" ]; then
  die "$link exists and is not a link: move it away, then run this again"
fi
ln -sfn "$INSTALL_DIR/current/bin/heavy-cmd" "$link"

case ":$PATH:" in
  *":$BIN_DIR:"*) ;;
  *) say "$BIN_DIR is not on PATH: add it to run heavy-cmd" ;;
esac
echo "heavy-cmd $tag installed: $link"
