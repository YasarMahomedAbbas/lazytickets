#!/bin/sh
# lazytickets installer — downloads the latest prebuilt binary for this machine.
#
#   curl --proto '=https' --tlsv1.2 -LsSf \
#     https://github.com/YasarMahomedAbbas/lazytickets/releases/latest/download/lazytickets-installer.sh | sh
#
# Re-running it upgrades an existing install to the latest release.
#
# Env overrides:
#   LAZYTICKETS_INSTALL_DIR   where to place the binary (default: ~/.local/bin)
#   LAZYTICKETS_VERSION       a specific tag, e.g. v0.2.0 (default: latest)
set -eu

REPO="YasarMahomedAbbas/lazytickets"
BIN="lazytickets"
INSTALL_DIR="${LAZYTICKETS_INSTALL_DIR:-$HOME/.local/bin}"
VERSION="${LAZYTICKETS_VERSION:-latest}"

err() { printf 'error: %s\n' "$1" >&2; exit 1; }
info() { printf '%s\n' "$1" >&2; }

# Only pull binaries when the tool the app depends on are present; warn otherwise.
command -v gh   >/dev/null 2>&1 || info "warning: 'gh' (GitHub CLI) not found on PATH — lazytickets needs it at runtime."
command -v tmux >/dev/null 2>&1 || info "warning: 'tmux' not found on PATH — lazytickets runs inside tmux."

# Map uname → a Rust target triple. Linux/musl only for now.
os="$(uname -s)"
arch="$(uname -m)"
[ "$os" = "Linux" ] || err "unsupported OS '$os' — only Linux has prebuilt binaries. Try: cargo install --git https://github.com/$REPO"

case "$arch" in
  x86_64 | amd64)          target="x86_64-unknown-linux-musl" ;;
  aarch64 | arm64)         target="aarch64-unknown-linux-musl" ;;
  *) err "unsupported architecture '$arch'. Try: cargo install --git https://github.com/$REPO" ;;
esac

if [ "$VERSION" = "latest" ]; then
  base="https://github.com/$REPO/releases/latest/download"
else
  base="https://github.com/$REPO/releases/download/$VERSION"
fi
archive="$BIN-$target.tar.gz"
url="$base/$archive"

command -v curl >/dev/null 2>&1 || err "curl is required."

tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT

info "Downloading $archive ..."
curl --proto '=https' --tlsv1.2 -fLsS "$url" -o "$tmp/$archive" \
  || err "download failed: $url"

# Best-effort checksum verification when the .sha256 sidecar is available.
if curl --proto '=https' --tlsv1.2 -fLsS "$url.sha256" -o "$tmp/$archive.sha256" 2>/dev/null; then
  if command -v sha256sum >/dev/null 2>&1; then
    expected="$(cut -d' ' -f1 < "$tmp/$archive.sha256")"
    actual="$(sha256sum "$tmp/$archive" | cut -d' ' -f1)"
    [ "$expected" = "$actual" ] || err "checksum mismatch — refusing to install."
    info "Checksum OK."
  fi
fi

tar -xzf "$tmp/$archive" -C "$tmp"
binpath="$(find "$tmp" -type f -name "$BIN" | head -n1)"
[ -n "$binpath" ] || err "binary '$BIN' not found in archive."

mkdir -p "$INSTALL_DIR"
install -m 0755 "$binpath" "$INSTALL_DIR/$BIN"
info "Installed $BIN → $INSTALL_DIR/$BIN"

case ":$PATH:" in
  *":$INSTALL_DIR:"*) ;;
  *) info "note: $INSTALL_DIR is not on your PATH. Add it, e.g.:"
     info "  echo 'set -gx PATH \$PATH $INSTALL_DIR' >> ~/.config/fish/config.fish" ;;
esac

info "Done. Run: $BIN"
