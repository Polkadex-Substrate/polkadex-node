# Polkadex Node — Package Build

Produces a `.deb` (Debian/Ubuntu) and `.rpm` (RHEL/Fedora/Rocky) package
for the `polkadex-node` binary.

## Quick start

```bash
# Build the release binary first (or pass --release to do it in one step)
cargo build --release -p polkadex-node

# Build both packages
./packaging/build-packages.sh

# Build only .deb
./packaging/build-packages.sh --deb

# Build only .rpm
./packaging/build-packages.sh --rpm

# Build binary + both packages in one step
./packaging/build-packages.sh --release
```

Output lands in `dist/`:
```
dist/polkadex-node_7.0.0_amd64.deb
dist/polkadex-node-7.0.0-1.x86_64.rpm
```

## Install

```bash
# Debian / Ubuntu
sudo dpkg -i dist/polkadex-node_*.deb

# RHEL / Fedora / Rocky
sudo rpm -i dist/polkadex-node-*.rpm
# or
sudo dnf install dist/polkadex-node-*.rpm
```

## Post-install configuration

1. Edit `/etc/polkadex/node.env` — set `NODE_NAME`, `VALIDATOR_FLAG="--validator"`
   if this node should validate (empty by default — installs as a full node),
   and `RPC_METHODS` if you need `safe` (e.g. an externally-facing archive
   node — see the comments in `node.env.example`).
2. `sudo systemctl enable --now polkadex-node`
3. `journalctl -u polkadex-node -f` — follow logs.
4. Verify you're on mainnet: block 0's hash must be
   `0x3920bcb4960a1eef5580cd5367ff3f430eef052774f78468852f7b9cb39f8a3c`
   (`curl -d '{"id":1,"jsonrpc":"2.0","method":"chain_getBlockHash","params":[0]}' http://127.0.0.1:9944`).

The chain spec lives at `/etc/polkadex/customSpecRaw.json`.  
Node data is stored under `/var/lib/polkadex` (owned by the `polkadex` system user).

Installing never auto-starts the service (`start = false` for deb,
no unconditional start in the rpm scriptlet) — step 2 above is required.
On a package **upgrade**, if the service was already running, it's restarted
automatically so the new binary actually takes effect; if it wasn't running,
it's left stopped.

## Files in this directory

| File | Purpose |
|---|---|
| `build-packages.sh` | Main build script |
| `deb/polkadex-node.service` | systemd unit — single source of truth, installed into both packages. Lives under `deb/` because cargo-deb's systemd integration requires it there; the rpm package references this same file. |
| `node.env.example` | Template for `/etc/polkadex/node.env` |
| `deb/postinst` | Debian post-install: creates user, dirs, copies env template |
| `deb/prerm` | Debian pre-remove: stops service |
| `deb/postrm` | Debian post-remove: purges config on `dpkg --purge` |
| `rpm/postinstall` | RPM post-install (`%post`): creates user, dirs, copies env template, `daemon-reload`, restarts the service on upgrade only if it was already running |
| `rpm/preuninstall` | RPM pre-uninstall (`%preun`): stops service, disables on real removal |
| `rpm/postuninstall` | RPM post-uninstall (`%postun`): purges config on real removal, `daemon-reload` |

Package metadata lives in `nodes/mainnet/Cargo.toml` under
`[package.metadata.deb]` and `[package.metadata.generate-rpm]`.

## Manual testing checklist (before the first tag)

Not covered by any automated check — needs real machines. Attach logs from
each run when reporting results.

- [ ] `dpkg -i` on a clean Ubuntu 22.04 install
- [ ] `dpkg -i` on a clean Ubuntu 24.04 install
- [ ] `dnf install` on a clean Rocky 9 install
- [ ] Node reaches peers and syncs on each of the above
- [ ] Node appears on telemetry under the `NODE_NAME` set in `node.env`, on
      Ubuntu 22.04 AND Rocky 9. An empty or default name means the
      `${NODE_NAME}` expansion in the unit failed
- [ ] Reinstall the same package over an existing install (`dpkg -i` /
      `dnf install` again) — confirm it doesn't break the running node, and
      end with `systemctl is-active polkadex-node`: a successful install that
      leaves the service stopped is a failure
- [ ] Upgrade to a newer package version over a **running** service — confirm
      the service actually restarts (not just that the binary on disk
      changed), confirm the version in the logs after restart matches
      the new package, not the old one, and end with
      `systemctl is-active polkadex-node` (on rpm the old package's `%preun`
      runs after the new `%post`, so this is where an unconditional stop
      would show)
- [ ] Upgrade over a package that was installed but never started — confirm
      it's still not running afterward (no surprise auto-start)
- [ ] Purge (`dpkg --purge` / equivalent) — confirm `/etc/polkadex` is
      removed, `/var/lib/polkadex` (chain data) is not
- [ ] One full run of `docs/migrate-to-packaged-node.md` end to end, starting
      from a box set up the old (manual/zip) way
