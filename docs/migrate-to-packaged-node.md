# Migrating an Existing Validator to the .deb/.rpm Package

For validators currently running `polkadex-node` from a source build, the
release zip (the setup in `docs/run-a-validator.md`) or the Docker image,
moving to the packaged install (`packaging/`) changes three things: the
binary's location, who owns the process, and where chain data lives. This
doc covers moving your existing node — keystore and database included —
without losing your session keys or re-syncing from genesis.

**Do this during low-risk maintenance time.** A validator that's offline
for the swap can miss blocks; there's no way around a short gap.

## What changes

| | Old (manual setup) | New (package) |
|---|---|---|
| Binary | wherever you put it (e.g. `$HOME/polkadex-node`) | `/usr/bin/polkadex-node` |
| Runs as | your own user | dedicated `polkadex` system user |
| Base path | `--base-path` if you set one, otherwise the Substrate default (`$HOME/.local/share/polkadex-node`); for the Docker image, the host directory mounted at `/data` | `/var/lib/polkadex` |
| Config | flags hardcoded in your own systemd unit | `/etc/polkadex/node.env` |
| Chain spec | wherever you downloaded it | `/etc/polkadex/customSpecRaw.json` |

The keystore (your session keys) lives inside the base path
(`<base-path>/chains/<chain-id>/keystore/`), so moving the base path moves
the keys with it — the package move itself needs no `author_rotateKeys` or
`setKeys`. The spec 392 runtime upgrade does, see "After the runtime upgrade"
at the end of this doc.

## Steps

1. **Find your current base path.** Check your existing systemd unit or
   startup command for `--base-path`. If it's not set explicitly, it
   defaults to `$HOME/.local/share/polkadex-node`.

2. **Stop and disable the old node.** Disabling matters: a stopped unit
   comes back on reboot and two nodes would then fight over the database.
   ```bash
   sudo systemctl disable --now <your-old-service-name>
   # or however you're currently running it
   ```
   Confirm it's actually stopped (`ps aux | grep polkadex-node`) before
   touching the data directory — copying a live RocksDB directory can
   corrupt it.

   Keep it stopped from here on. The old and the new node must never run at
   the same time: they hold the same session keys, so both would author blocks
   and sign GRANDPA votes, and double-signing is slashed.

3. **Install the package.** This creates the `polkadex` user and
   `/var/lib/polkadex` (empty) via the postinstall script, but does **not**
   start the service yet.
   ```bash
   sudo apt install ./polkadex-node_*.deb      # or: sudo dnf install ./polkadex-node-*.rpm
   ```
   apt and dnf also install anything the package needs that is missing. The
   package needs glibc 2.34 or newer: Ubuntu 22.04, Debian 12, Rocky 9 or later.

4. **Copy your existing chain data into the new location.**
   ```bash
   sudo rsync -a --delete <old-base-path>/ /var/lib/polkadex/
   sudo chown -R polkadex:polkadex /var/lib/polkadex
   ```
   `--delete` clears the empty scaffold the package created first, so you
   don't end up with a stray empty `chains/` next to your real one — only
   use it because `/var/lib/polkadex` is freshly created and has nothing
   worth keeping yet.

   On a server with SELinux enforcing (Rocky, RHEL), reset the file labels
   after copying:
   ```bash
   sudo restorecon -R /var/lib/polkadex /etc/polkadex
   ```

   Moving to a different server works the same way: copy `<old-base-path>`
   across (for example with `rsync -a` over ssh) once the old node is stopped
   and disabled, then continue with the steps below. The database, the
   keystore and the `network` directory (your node key, which keeps your peer
   ID) all live under `chains/<chain-id>/` and move together.

5. **Configure the service.**
   ```bash
   sudo $EDITOR /etc/polkadex/node.env
   ```
   The package creates this file from
   `/usr/share/polkadex-node/node.env.example` when it is installed.
   Set `NODE_NAME` to match what you had before (keeps your telemetry
   history recognizable), and set `VALIDATOR_FLAG="--validator"` — this is
   not the default, and if you skip it the node comes up as a full node
   instead of a validator. Leave the flags the service unit already sets out
   of `EXTRA_FLAGS`; the comments in `node.env` list them.

6. **Start and verify.**
   ```bash
   sudo systemctl enable --now polkadex-node
   journalctl -u polkadex-node -f
   ```
   Confirm:
   - The log shows `Role: AUTHORITY` and `Starting BABE Authorship worker` at
     startup, then `Pre-sealed block for proposal` whenever the node authors
     a block.
   - The node reports the same highest block you left off at (no resync
     from genesis — if it's resyncing, the data copy didn't take).
   - The genesis hash matches mainnet's:
     `curl -H "Content-Type:application/json" -d '{"id":1,"jsonrpc":"2.0","method":"chain_getBlockHash","params":[0]}' http://127.0.0.1:9944`
     must return
     `0x3920bcb4960a1eef5580cd5367ff3f430eef052774f78468852f7b9cb39f8a3c` —
     if it doesn't, the copied data directory isn't mainnet's, or
     `customSpecRaw.json` doesn't match what you were running before.
   - `curl -H "Content-Type:application/json" -d '{"id":1,"jsonrpc":"2.0","method":"author_hasSessionKeys","params":["<your session key bytes>"]}' http://127.0.0.1:9944`
     confirms the keystore carried over.
   - Your validator resumes producing/backing blocks in the following
     sessions (check telemetry or `polkadot.js apps` staking tab).

7. **Clean up.** Once you've confirmed the new setup is stable for a few
   sessions, remove the old binary/systemd unit and old data directory.
   Don't delete the old data directory immediately after cutover — keep it
   as a rollback path until you're confident the migration held.

## After the runtime upgrade (spec 392)

The spec 392 upgrade adds two new session key types (BEEFY and mixnet). The
upgrade migration fills them with placeholder values for every validator. Your
existing BABE and GRANDPA keys keep working, but rotate once after the upgrade
so those two entries hold real keys:

1. Wait until the runtime upgrade has enacted (the node logs the new spec
   version, or `state_getRuntimeVersion` shows 392).
2. Generate new keys on the node:
   ```bash
   curl -H "Content-Type: application/json" \
     -d '{"id":1,"jsonrpc":"2.0","method":"author_rotateKeys","params":[]}' \
     http://127.0.0.1:9944
   ```
3. Submit the returned hex with `session.setKeys`, signed by your **stash**
   account (polkadot.js apps, Developer > Extrinsics), with `0x` as `proof`.
   From 392 a `setKeys` signed by a separate controller account succeeds but
   does not set your validator's keys.
4. The new keys become active from the session after next. Until then the
   node validates with its existing BABE and GRANDPA keys, nothing is lost.
   Keep the old keys on the node until then.

Rotating before the upgrade enacts produces keys in the old format, which the
upgrade then pads with placeholders, so you would have to rotate again. Wait
for enactment.

## Rollback

If something's wrong after step 6, stop the new service (`sudo systemctl
stop polkadex-node`) and only then restart your old setup, never both at
once (see step 2). The old base path is untouched by this process (steps 4
only copies *from* it), so nothing about the migration is destructive to
your original data.
