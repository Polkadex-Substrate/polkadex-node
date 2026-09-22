# Migrating an Existing Validator to the .deb/.rpm Package

For validators currently running `polkadex-node` from a source build or the
release zip (the setup in `docs/run-a-validator.md`), moving to the packaged
install (`packaging/`) changes three things: the binary's location, who owns
the process, and where chain data lives. This doc covers moving your existing
node — keystore and database included — without losing your session keys or
re-syncing from genesis.

**Do this during low-risk maintenance time.** A validator that's offline
for the swap can miss blocks; there's no way around a short gap.

## What changes

| | Old (manual setup) | New (package) |
|---|---|---|
| Binary | wherever you put it (e.g. `$HOME/polkadex-node`) | `/usr/local/bin/polkadex-node` |
| Runs as | your own user | dedicated `polkadex` system user |
| Base path | `--base-path` if you set one, otherwise the Substrate default (`$HOME/.local/share/polkadex-node`) | `/var/lib/polkadex` |
| Config | flags hardcoded in your own systemd unit | `/etc/polkadex/node.env` |
| Chain spec | wherever you downloaded it | `/etc/polkadex/customSpecRaw.json` |

The keystore (your session keys) lives inside the base path
(`<base-path>/chains/<chain-id>/keystore/`), so moving the base path moves
the keys with it — no need to re-run `author_rotateKeys` or resubmit
`setKeys` afterward.

## Steps

1. **Find your current base path.** Check your existing systemd unit or
   startup command for `--base-path`. If it's not set explicitly, it
   defaults to `$HOME/.local/share/polkadex-node`.

2. **Stop the old node.**
   ```bash
   sudo systemctl stop <your-old-service-name>
   # or however you're currently running it
   ```
   Confirm it's actually stopped (`ps aux | grep polkadex-node`) before
   touching the data directory — copying a live RocksDB directory can
   corrupt it.

3. **Install the package.** This creates the `polkadex` user and
   `/var/lib/polkadex` (empty) via the postinstall script, but does **not**
   start the service yet.
   ```bash
   sudo dpkg -i polkadex-node_*.deb      # or: sudo rpm -i polkadex-node-*.rpm
   ```

4. **Copy your existing chain data into the new location.**
   ```bash
   sudo rsync -a --delete <old-base-path>/ /var/lib/polkadex/
   sudo chown -R polkadex:polkadex /var/lib/polkadex
   ```
   `--delete` clears the empty scaffold the package created first, so you
   don't end up with a stray empty `chains/` next to your real one — only
   use it because `/var/lib/polkadex` is freshly created and has nothing
   worth keeping yet.

5. **Configure the service.**
   ```bash
   sudo cp /usr/share/doc/polkadex-node/node.env.example /etc/polkadex/node.env
   sudo $EDITOR /etc/polkadex/node.env
   ```
   Set `NODE_NAME` to match what you had before (keeps your telemetry
   history recognizable), and set `VALIDATOR_FLAG="--validator"` — this is
   not the default, and if you skip it the node comes up as a full node
   instead of a validator.

6. **Start and verify.**
   ```bash
   sudo systemctl enable --now polkadex-node
   journalctl -u polkadex-node -f
   ```
   Confirm:
   - The node reports the same highest block you left off at (no resync
     from genesis — if it's resyncing, the data copy didn't take).
   - `curl -H "Content-Type:application/json" -d '{"id":1,"jsonrpc":"2.0","method":"author_hasSessionKeys","params":["<your session key bytes>"]}' http://localhost:9944`
     confirms the keystore carried over.
   - Your validator resumes producing/backing blocks in the following
     sessions (check telemetry or `polkadot.js apps` staking tab).

7. **Clean up.** Once you've confirmed the new setup is stable for a few
   sessions, remove the old binary/systemd unit and old data directory.
   Don't delete the old data directory immediately after cutover — keep it
   as a rollback path until you're confident the migration held.

## Rollback

If something's wrong after step 6, stop the new service
(`sudo systemctl stop polkadex-node`) and restart your old setup — the old
base path is untouched by this process (steps 4 only copies *from* it), so
nothing about the migration is destructive to your original data.
