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

1. Edit `/etc/polkadex/node.env` — set `NODE_NAME`, and `VALIDATOR_FLAG="--validator"`
   if this node should validate (empty by default — installs as a full node).
2. `sudo systemctl enable --now polkadex-node`
3. `journalctl -u polkadex-node -f` — follow logs.

The chain spec lives at `/etc/polkadex/customSpecRaw.json`.  
Node data is stored under `/var/lib/polkadex` (owned by the `polkadex` system user).

## Files in this directory

| File | Purpose |
|---|---|
| `build-packages.sh` | Main build script |
| `deb/polkadex-node.service` | systemd unit — single source of truth, installed into both packages. Lives under `deb/` because cargo-deb's systemd integration requires it there; the rpm package references this same file. |
| `node.env.example` | Template for `/etc/polkadex/node.env` |
| `deb/postinst` | Debian post-install: creates user, dirs, copies env template |
| `deb/prerm` | Debian pre-remove: stops service |
| `deb/postrm` | Debian post-remove: purges config on `dpkg --purge` |
| `rpm/postinstall` | RPM post-install (`%post`): creates user, dirs, copies env template, `daemon-reload` |
| `rpm/preuninstall` | RPM pre-uninstall (`%preun`): stops service, disables on real removal |
| `rpm/postuninstall` | RPM post-uninstall (`%postun`): purges config on real removal, `daemon-reload` |

Package metadata lives in `nodes/mainnet/Cargo.toml` under
`[package.metadata.deb]` and `[package.metadata.generate-rpm]`.
