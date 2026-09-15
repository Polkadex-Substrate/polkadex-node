#!/usr/bin/env bash
# packaging/build-packages.sh
# Build Debian (.deb) and RPM (.rpm) packages for polkadex-node.
#
# Usage:
#   ./packaging/build-packages.sh [--deb] [--rpm] [--release]
#
# Flags:
#   --deb      Build .deb only
#   --rpm      Build .rpm only
#   (neither)  Build both
#   --release  Run `cargo build --release` first (skip if binary already exists)
#
# Output:
#   dist/polkadex-node_<version>_amd64.deb
#   dist/polkadex-node-<version>-1.x86_64.rpm
#
# Requirements (installed automatically if missing):
#   cargo-deb           https://github.com/kornelski/cargo-deb
#   cargo-generate-rpm  https://github.com/cat-in-136/cargo-generate-rpm

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
DIST_DIR="$REPO_ROOT/dist"
NODE_MANIFEST="$REPO_ROOT/nodes/mainnet/Cargo.toml"
BINARY="$REPO_ROOT/target/release/polkadex-node"

BUILD_DEB=true
BUILD_RPM=true
BUILD_BINARY=false

# ── Parse arguments ────────────────────────────────────────────────────────
for arg in "$@"; do
  case "$arg" in
    --deb)     BUILD_RPM=false ;;
    --rpm)     BUILD_DEB=false ;;
    --release) BUILD_BINARY=true ;;
    --help|-h)
      sed -n '2,20p' "$0" | sed 's/^# //'
      exit 0 ;;
    *) echo "Unknown flag: $arg" >&2; exit 1 ;;
  esac
done

mkdir -p "$DIST_DIR"

# ── Helpers ────────────────────────────────────────────────────────────────
info()  { echo "  [+] $*"; }
warn()  { echo "  [!] $*" >&2; }
die()   { echo "  [✗] $*" >&2; exit 1; }

ensure_cargo_plugin() {
  local plugin="$1"
  if ! cargo "$plugin" --version > /dev/null 2>&1; then
    info "Installing cargo-$plugin …"
    cargo install "cargo-$plugin" --locked
  fi
}

# ── 1. Build the release binary ────────────────────────────────────────────
if $BUILD_BINARY; then
  info "Building polkadex-node --release …"
  (cd "$REPO_ROOT" && cargo build --release -p polkadex-node)
fi

if [ ! -f "$BINARY" ]; then
  die "Release binary not found at $BINARY. Run with --release or build manually first."
fi

VERSION=$(file "$BINARY" | head -1 || true)
info "Binary : $BINARY"

# ── 2. Debian package ──────────────────────────────────────────────────────
if $BUILD_DEB; then
  ensure_cargo_plugin "deb"

  info "Building .deb …"
  # cargo-deb must run from the workspace root so it can find the binary
  DEB_PATH=$(
    cd "$REPO_ROOT" && \
    cargo deb \
      --manifest-path "$NODE_MANIFEST" \
      --no-build \
      --output "$DIST_DIR" \
      2>&1 | tee /dev/stderr | grep -oE '[^ ]+\.deb$' | tail -1
  )

  if [ -n "$DEB_PATH" ] && [ -f "$DEB_PATH" ]; then
    info ".deb : $DEB_PATH"
    # Show package info
    dpkg-deb --info "$DEB_PATH" 2>/dev/null | grep -E "Package|Version|Architecture|Installed-Size" || true
  else
    warn ".deb output path not detected — check $DIST_DIR manually."
  fi
fi

# ── 3. RPM package ─────────────────────────────────────────────────────────
if $BUILD_RPM; then
  ensure_cargo_plugin "generate-rpm"

  info "Building .rpm …"
  cd "$REPO_ROOT"
  cargo generate-rpm \
    --manifest-path "$NODE_MANIFEST" \
    --output "$DIST_DIR" \
    2>&1 | tee /dev/stderr || true

  RPM_PATH=$(find "$DIST_DIR" -name "*.rpm" -newer "$BINARY" 2>/dev/null | head -1)
  if [ -n "$RPM_PATH" ] && [ -f "$RPM_PATH" ]; then
    info ".rpm : $RPM_PATH"
    rpm -qip "$RPM_PATH" 2>/dev/null | grep -E "^Name|^Version|^Architecture|^Size" || true
  else
    warn ".rpm output path not detected — check $DIST_DIR manually."
  fi
fi

echo ""
info "Done. Packages in $DIST_DIR/"
ls -lh "$DIST_DIR/"*.deb "$DIST_DIR/"*.rpm 2>/dev/null || true
