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
  local plugin="$1" version="$2"
  if ! cargo "$plugin" --version 2>/dev/null | grep -q "$version"; then
    info "Installing cargo-$plugin $version …"
    cargo install "cargo-$plugin" --version "$version" --locked
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

info "Binary  : $BINARY ($(du -sh "$BINARY" | cut -f1))"

# Note: no manual strip step here — cargo-deb strips debug symbols itself by
# default (strip_override defaults to allowing it), so stripping the binary
# ourselves first was redundant.

# ── 2. Debian package ──────────────────────────────────────────────────────
if $BUILD_DEB; then
  ensure_cargo_plugin "deb" "3.8.0"

  info "Building .deb …"
  # Run from the workspace root so cargo-deb can locate workspace members.
  # --manifest-path points at the node crate; --no-build skips compilation.
  # --output sets the destination directory for the produced .deb file.
  # The `|| true` matters: under `pipefail`, if cargo-deb's output consists
  # entirely of the filtered line, grep -v finds nothing to print and exits
  # 1, which would abort this script even though cargo-deb itself succeeded.
  (cd "$REPO_ROOT" && cargo deb \
      --manifest-path "$NODE_MANIFEST" \
      --no-build \
      --output "$DIST_DIR/" 2>&1 | { grep -v "Only source paths starting with" || true; })

  DEB_PATH=$(find "$DIST_DIR" -name "*.deb" -newer "$BINARY" 2>/dev/null | sort | tail -1)
  if [ -n "$DEB_PATH" ] && [ -f "$DEB_PATH" ]; then
    info ".deb    : $DEB_PATH ($(du -sh "$DEB_PATH" | cut -f1))"
    dpkg-deb --info "$DEB_PATH" 2>/dev/null \
      | grep -E "Package|Version|Architecture|Installed-Size" || true
  else
    warn "Could not locate the produced .deb in $DIST_DIR — check output above."
  fi
fi

# ── 3. RPM package ─────────────────────────────────────────────────────────
if $BUILD_RPM; then
  ensure_cargo_plugin "generate-rpm" "0.21.0"

  info "Building .rpm …"
  # cargo-generate-rpm must run from the crate directory (it doesn't support --manifest-path
  # or workspace -p resolution). Asset paths in Cargo.toml are relative to nodes/mainnet/.
  (cd "$REPO_ROOT/nodes/mainnet" && cargo generate-rpm \
      --output "$DIST_DIR/")

  RPM_PATH=$(find "$DIST_DIR" -name "*.rpm" -newer "$BINARY" 2>/dev/null | sort | tail -1)
  if [ -n "$RPM_PATH" ] && [ -f "$RPM_PATH" ]; then
    info ".rpm    : $RPM_PATH ($(du -sh "$RPM_PATH" | cut -f1))"
    rpm -qip "$RPM_PATH" 2>/dev/null \
      | grep -E "^Name|^Version|^Architecture|^Size" || true
  else
    warn "Could not locate the produced .rpm in $DIST_DIR — check output above."
  fi
fi

echo ""
info "Done. Packages in $DIST_DIR/"
ls -lh "$DIST_DIR/"*.deb "$DIST_DIR/"*.rpm 2>/dev/null || true
