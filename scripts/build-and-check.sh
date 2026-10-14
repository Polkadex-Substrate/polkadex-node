#!/usr/bin/env bash
# Build and check the mainnet runtime on a fresh Ubuntu 24.04 machine.
#
#     bash ~/polkadex-node/scripts/build-and-check.sh
#
# Expects the repository at ~/polkadex-node (override with REPO=...). In order it:
#   1. installs the build dependencies (protoc and rustup-init pinned as in release.yml) and the
#      toolchain pinned in rust-toolchain.toml
#   2. builds the release runtime wasm (compact, compressed) with on-chain-release-build
#   3. runs the runtime unit tests
#   4. builds the runtime with the try-runtime feature
#   5. downloads a pinned try-runtime-cli release and checks its hash
#   6. runs `on-runtime-upgrade --checks all --disable-mbm-checks` live against mainnet, and
#      against a snapshot taken from the same endpoint if the live run fails
#
# SNAPSHOT_FILE=... (optional) saves the live state download: a `try-runtime create-snapshot`
# started separately, before this script, writes the snapshot there and its exit code to
# "$SNAPSHOT_FILE.exit" when it finishes. The on-runtime-upgrade step waits for that and runs
# against the snapshot, falling back to the live run if the snapshot failed. The separate run must
# use its own copy of the try-runtime binary, since the download step rewrites ~/tools/try-runtime.
#
# Every step writes <step>.log and <step>.exit to ~/results (override with RESULTS=...). A step
# that fails does not stop the steps that do not depend on it, so one run reports everything.
# The run ends with ~/results/SUMMARY.txt (step results, wasm hashes, test and try-runtime
# lines) and then ~/results/DONE.

set -uo pipefail

REPO="${REPO:-$HOME/polkadex-node}"
RESULTS="${RESULTS:-$HOME/results}"
URI="${URI:-wss://rpc.polkadex.ee}"
JOBS="${JOBS:-$(nproc)}"

# protoc and rustup-init: the pins and hashes used by .github/workflows/release.yml.
PROTOC_VER=25.3
PROTOC_SHA256=f853e691868d0557425ea290bf7ba6384eef2fa9b04c323afab49a770ba9da80
RUSTUP_VER=1.29.1
RUSTUP_SHA256=dda7234360b7f578ca8b0ddcb80145646fa61a67c1720a5abc7051b35c9fcb71
# try-runtime-cli: the release used for the earlier 392 checks
# (docs/try-runtime-results-mainnet.md). The hash is the digest GitHub lists for the asset.
TRY_RUNTIME_VER=v0.10.1
TRY_RUNTIME_SHA256=d6fcd586fba4c7245668f4ae2a06ffc94e6da4135fae22cfe24453dd11aee3c3

WBUILD="$REPO/target/release/wbuild/node-polkadex-runtime"
WASM_NAME=node_polkadex_runtime.compact.compressed.wasm
RELEASE_WASM="$RESULTS/node_polkadex_runtime-392.compact.compressed.wasm"
TRY_WASM="$RESULTS/node_polkadex_runtime-392-tryruntime.compact.compressed.wasm"
TOOLS="$HOME/tools"
SNAPSHOT="$HOME/snapshot/mainnet.snap"
SNAPSHOT_FILE="${SNAPSHOT_FILE:-}"
LOCKED="--locked"

export PATH="$HOME/.cargo/bin:$TOOLS:$PATH"
export CARGO_NET_GIT_FETCH_WITH_CLI=true
export DEBIAN_FRONTEND=noninteractive
SUDO=""
[ "$(id -u)" -ne 0 ] && SUDO="sudo"

mkdir -p "$RESULTS" "$TOOLS"
rm -f "$RESULTS/DONE"
STEPS=()

# step NAME FUNCTION: runs FUNCTION in $REPO, output to NAME.log, exit code to NAME.exit.
step() {
	local name="$1" start rc
	shift
	STEPS+=("$name")
	start=$(date +%s)
	{
		echo "== $name"
		echo "== started $(date -u +%FT%TZ)"
		echo "== running: $*"
	} > "$RESULTS/$name.log"
	(cd "$REPO" || exit 97; "$@") >> "$RESULTS/$name.log" 2>&1
	rc=$?
	echo "$rc" > "$RESULTS/$name.exit"
	echo "== exit $rc after $(($(date +%s) - start))s" >> "$RESULTS/$name.log"
	echo "$name: exit $rc ($(($(date +%s) - start))s)"
	return "$rc"
}

# skip NAME REASON: records a step that did not run because one it needs failed.
skip() {
	STEPS+=("$1")
	echo "skipped: $2" > "$RESULTS/$1.log"
	echo "skipped" > "$RESULTS/$1.exit"
	echo "$1: skipped ($2)"
}

ok() {
	[ "$(cat "$RESULTS/$1.exit" 2>/dev/null)" = "0" ]
}

hashes() {
	local f="$1"
	echo "file:       $(basename "$f")"
	echo "bytes:      $(stat -c %s "$f")"
	echo "sha256:     $(sha256sum "$f" | cut -d' ' -f1)"
	# blake2_256 is the hash the chain uses for :code (authorize_upgrade, code hash in logs).
	echo "blake2_256: 0x$(b2sum -l 256 "$f" | cut -d' ' -f1)"
	python3 -c 'import hashlib, sys; print("blake2_256 (python check): 0x" + hashlib.blake2b(open(sys.argv[1], "rb").read(), digest_size=32).hexdigest())' "$f"
}

environment() {
	uname -a
	grep PRETTY_NAME /etc/os-release
	echo "cpus: $(nproc)"
	free -g
	df -h "$HOME"
	git -C "$REPO" log -1 --format='commit %H%n%s' 2>/dev/null || echo "no git metadata in $REPO"
	git -C "$REPO" status --short 2>/dev/null | head -50
}

system_deps() {
	set -e
	$SUDO apt-get update -q
	$SUDO apt-get install -y -q build-essential clang libclang-dev llvm-dev cmake pkg-config \
		git curl unzip ca-certificates make perl python3
	curl -sSfL -o /tmp/protoc.zip \
		"https://github.com/protocolbuffers/protobuf/releases/download/v${PROTOC_VER}/protoc-${PROTOC_VER}-linux-x86_64.zip"
	echo "${PROTOC_SHA256}  /tmp/protoc.zip" | sha256sum -c -
	$SUDO unzip -q -o /tmp/protoc.zip -d /usr/local bin/protoc 'include/*'
	protoc --version
	curl --proto '=https' --tlsv1.2 -sSfL -o /tmp/rustup-init \
		"https://static.rust-lang.org/rustup/archive/${RUSTUP_VER}/x86_64-unknown-linux-gnu/rustup-init"
	echo "${RUSTUP_SHA256}  /tmp/rustup-init" | sha256sum -c -
	chmod +x /tmp/rustup-init
	# No default toolchain: rust-toolchain.toml in the repository decides.
	/tmp/rustup-init -y --no-modify-path --default-toolchain none
	rm -f /tmp/rustup-init
	git config --global --add safe.directory "$REPO"
}

toolchain() {
	set -e
	# Installs the channel, components and targets named in rust-toolchain.toml.
	rustup toolchain install || rustup show
	rustup show
	rustc -Vv
	cargo -V
	rustup target list --installed
}

lockfile() {
	# Cargo.lock was edited by hand when crates were dropped; --locked fails if it does not
	# match the manifests.
	cargo metadata --locked --format-version 1 > /dev/null && echo "Cargo.lock matches the manifests"
}

lockfile_regenerate() {
	set -e
	cp Cargo.lock "$RESULTS/Cargo.lock.before"
	cargo metadata --format-version 1 > /dev/null
	cp Cargo.lock "$RESULTS/Cargo.lock.after"
	diff -u "$RESULTS/Cargo.lock.before" Cargo.lock > "$RESULTS/Cargo.lock.diff" || true
	echo "Cargo.lock regenerated; the change is in $RESULTS/Cargo.lock.diff"
	cat "$RESULTS/Cargo.lock.diff"
}

release_wasm() {
	set -e
	# The release build (docs/migrations.md): metadata hash included for CheckMetadataHash.
	cargo build --release $LOCKED -j "$JOBS" -p node-polkadex-runtime --features on-chain-release-build
	cp "$WBUILD/$WASM_NAME" "$RELEASE_WASM"
	hashes "$RELEASE_WASM" | tee "$RESULTS/wasm-hashes.txt"
}

runtime_tests() {
	SKIP_WASM_BUILD=1 cargo test --release $LOCKED -j "$JOBS" -p node-polkadex-runtime --no-fail-fast
}

try_runtime_wasm() {
	set -e
	cargo build --release $LOCKED -j "$JOBS" -p node-polkadex-runtime --features try-runtime
	cp "$WBUILD/$WASM_NAME" "$TRY_WASM"
	hashes "$TRY_WASM" | tee "$RESULTS/wasm-tryruntime-hashes.txt"
}

try_runtime_cli() {
	set -e
	curl -sSfL -o "$TOOLS/try-runtime" \
		"https://github.com/paritytech/try-runtime-cli/releases/download/${TRY_RUNTIME_VER}/try-runtime-x86_64-unknown-linux-musl"
	echo "${TRY_RUNTIME_SHA256}  $TOOLS/try-runtime" | sha256sum -c -
	chmod +x "$TOOLS/try-runtime"
	try-runtime --version
}

# --disable-mbm-checks: with try-runtime-cli 0.10.1 the first pass, the only one that runs the
# migrations on unmigrated state, gets --checks only when this flag is set. This runtime has no
# multi-block migrations, so it skips nothing (docs/try-runtime-results-mainnet.md).
upgrade_live() {
	RUST_LOG=info,runtime=info try-runtime --runtime "$TRY_WASM" \
		on-runtime-upgrade --blocktime 12000 --checks all --disable-mbm-checks \
		live --uri "$URI"
}

upgrade_prepared_snapshot() {
	local waited=0
	echo "== waiting for $SNAPSHOT_FILE.exit"
	while [ ! -f "$SNAPSHOT_FILE.exit" ] && [ "$waited" -lt 5400 ]; do
		sleep 30
		waited=$((waited + 30))
	done
	[ "$(cat "$SNAPSHOT_FILE.exit" 2>/dev/null)" = "0" ] && [ -s "$SNAPSHOT_FILE" ] || {
		echo "no usable snapshot after ${waited}s"
		return 1
	}
	ls -l "$SNAPSHOT_FILE"
	RUST_LOG=info,runtime=info try-runtime --runtime "$TRY_WASM" \
		on-runtime-upgrade --blocktime 12000 --checks all --disable-mbm-checks \
		snap --path "$SNAPSHOT_FILE"
}

upgrade_snapshot() {
	local attempt
	mkdir -p "$(dirname "$SNAPSHOT")"
	for attempt in 1 2 3; do
		rm -f "$SNAPSHOT"
		echo "== create-snapshot attempt $attempt"
		try-runtime create-snapshot "$SNAPSHOT" --uri "$URI" && break
		sleep 60
	done
	[ -s "$SNAPSHOT" ] || { echo "no snapshot"; return 1; }
	ls -l "$SNAPSHOT"
	RUST_LOG=info,runtime=info try-runtime --runtime "$TRY_WASM" \
		on-runtime-upgrade --blocktime 12000 --checks all --disable-mbm-checks \
		snap --path "$SNAPSHOT"
}

summary() {
	local s f
	{
		echo "build-and-check finished $(date -u +%FT%TZ)"
		echo "repo: $REPO $(git -C "$REPO" rev-parse HEAD 2>/dev/null || echo '(no git metadata)')"
		echo
		for s in "${STEPS[@]}"; do
			printf '%-28s %s\n' "$s" "$(cat "$RESULTS/$s.exit" 2>/dev/null || echo '?')"
		done
		echo
		if [ "$LOCKED" != "--locked" ]; then
			echo "NOTE: Cargo.lock did not match the manifests. The builds ran without --locked;"
			echo "      the needed Cargo.lock change is in Cargo.lock.diff."
			echo
		fi
		echo "== release wasm (on-chain-release-build)"
		cat "$RESULTS/wasm-hashes.txt" 2>/dev/null || echo "not built"
		echo
		echo "== try-runtime wasm"
		cat "$RESULTS/wasm-tryruntime-hashes.txt" 2>/dev/null || echo "not built"
		echo
		echo "== runtime tests"
		grep -E '^test result:|^test .* FAILED$|panicked at|^error(\[|:)' "$RESULTS/05-runtime-tests.log" 2>/dev/null | head -60
		echo
		echo "== try-runtime"
		for f in "$RESULTS"/08*.log; do
			[ -f "$f" ] || continue
			echo "-- $(basename "$f")"
			grep -E 'Original runtime|New runtime|ClearRewardsAndMigrationLocks|ClearOrmlVestingLocks|bond|pallet-treasury|idempotent|weight safety|Consumed ref_time|PoV size|decodes without error|try-state|ERROR|panicked|failed' "$f" | head -80
		done
		echo
		df -h "$HOME" | tail -1
	} > "$RESULTS/SUMMARY.txt"
	date -u +%FT%TZ > "$RESULTS/DONE"
}

step 00-environment environment

step 01-system-deps system_deps
if ok 01-system-deps; then
	step 02-toolchain toolchain
else
	skip 02-toolchain "01-system-deps failed"
fi

if ok 02-toolchain; then
	if ! step 03-lockfile lockfile; then
		step 03b-lockfile-regenerate lockfile_regenerate
		LOCKED=""
	fi
	step 04-release-wasm release_wasm
	step 05-runtime-tests runtime_tests
	step 06-try-runtime-wasm try_runtime_wasm
else
	for s in 03-lockfile 04-release-wasm 05-runtime-tests 06-try-runtime-wasm; do
		skip "$s" "02-toolchain failed"
	done
fi

if ok 01-system-deps; then
	step 07-try-runtime-cli try_runtime_cli
else
	skip 07-try-runtime-cli "01-system-deps failed"
fi

if ok 06-try-runtime-wasm && ok 07-try-runtime-cli; then
	if [ -n "$SNAPSHOT_FILE" ] && step 08-try-runtime-snapshot upgrade_prepared_snapshot; then
		:
	elif ! step 08-try-runtime-live upgrade_live; then
		step 08b-try-runtime-snapshot upgrade_snapshot
	fi
else
	skip 08-try-runtime-live "the try-runtime wasm or try-runtime-cli is missing"
fi

summary
echo "done: $RESULTS/SUMMARY.txt"
