## Install

Download the `.deb` or `.rpm` from the **Assets** below, then:

**Debian / Ubuntu:**
```bash
sudo dpkg -i polkadex-node_*.deb
```

**RHEL / Fedora / Rocky:**
```bash
sudo rpm -i polkadex-node-*.rpm
```

Then edit `/etc/polkadex/node.env` (set `NODE_NAME`, and
`VALIDATOR_FLAG="--validator"` if this node should validate — it's empty by
default) and start it:
```bash
sudo systemctl enable --now polkadex-node
journalctl -u polkadex-node -f
```

Full instructions: `packaging/README.md`.
Moving an existing validator to this package: `docs/migrate-to-packaged-node.md`.

**Verify your download** — download `SHA256SUMS` from Assets too, then:
```bash
sha256sum -c SHA256SUMS
```

---
