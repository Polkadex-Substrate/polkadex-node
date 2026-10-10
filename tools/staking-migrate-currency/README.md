# Staking sweep after the 392 upgrade: `migrate_currency.py`

From spec 392, pallet-staking keeps bonded PDEX on a hold instead of the old `staking ` lock. Each stash
switches lazily, at its first ledger update, and a hold must leave the existential deposit (1 PDEX)
free. A stash whose free balance minus 1 PDEX is below its bond (`ledger.total`) cannot hold its whole
bond:

- any ledger update on it fails with `Staking.NotEnoughFunds`;
- `payout_stakers` updates the validator's ledger before it pays anyone, so a payout for such a
  validator fails for the validator and for all of its nominators;
- for such a nominator with rewards restaked, the payout itself succeeds, but pallet-staking 45.1.0
  leaves the reward unbonded in the free balance and logs a `runtime::defensive` error on every node.

`staking.migrateCurrency(stash)` fixes a stash. It is permissionless and its fee is refunded when it
succeeds. It removes the lock and holds the bond instead. If the stash cannot hold all of it, the
ledger is lowered to what it can hold (free minus 1 PDEX), from `active` first and then from the
unlocking chunks, and the call emits `Staking.CurrencyMigrated { stash, force_withdraw }`.
Nothing leaves the stash; the trimmed part is just no longer bonded.

On mainnet state of 10 Oct 2026: 1,625 of 3,410 stashes cannot hold their full bond (75 validators,
1,144 nominators, 406 bonded but idle). Total trim 1,135.16 PDEX, at most 74.81 PDEX on one stash.
73 validators end below the 100 PDEX `MinValidatorBond`: 71 of them by at most 1 PDEX, 2 by about
65.5 PDEX.

The script sends only `Staking.migrate_currency` calls.

## Requirements

- Python 3.9 or later and `pip install substrate-interface==1.8.1`, the version it was rehearsed with.
- The websocket RPC endpoint of a synced mainnet node the team runs. The script uses
  `author_submitExtrinsic`, `author_pendingExtrinsics` and `system_accountNextIndex`, all served with
  the default RPC methods.
- A signer: a dedicated ops account holding about 10 PDEX and nothing else. The call is permissionless,
  so the account needs no role. Successful calls cost nothing; the balance only pays for calls that
  fail. The script refuses to start below 2 PDEX.
- A signer file with that account's mnemonic or secret URI (sr25519), mode 600, on the ops machine
  only. The script reads it and never prints or logs it.

## Run

Dry run first. It needs no signer and sends nothing:

```bash
python3 migrate_currency.py --rpc ws://127.0.0.1:9944 --dry-run > dryrun.txt
```

One line per target stash: stash, role, bonded, active, free, expected trim, active bond after, and
`BELOW_MIN(top_up X)` for a validator that ends below `MinValidatorBond`. The `# summary` line at the
end has the totals.

Then the sweep:

```bash
python3 migrate_currency.py --rpc ws://127.0.0.1:9944 --signer-file ~/ops-signer.secret \
  --out sweep.jsonl > sweep.log 2>&1
```

Options:

- `--all-locked` also migrates the stashes that can hold their full bond (no trim). Payouts do not
  need it: those stashes switch by themselves at their next ledger update.
- `--wave-size N`: calls per wave, default 50.
- `--out FILE`: the JSONL output, default `migrate_currency-<UTC time>.jsonl`. The summary goes to
  `FILE.summary.json`.

How it sends: calls are signed with explicit nonces and a 64-block mortality, 50 to a wave, one wave
per block. It waits until the wave is in a block, then until that block is finalized, and reads the
results from the finalized blocks. A call the pool dropped is sent again alone, after an on-chain
check that the stash is still on the staking lock; the new call takes the dropped call's nonce, so
the two cannot both land. Stashes that left the lock before their wave are skipped. The script stops
when the pool rejects the first call of a wave, when every call of a wave fails, or when finality
stalls for 15 minutes. Running it again is safe: migrated stashes are no longer targets.

## Output

One JSONL line per stash: `status` (`ok`, `error` or `skipped`), the error name, block, extrinsic
index and hash, nonce, attempt, `fee` charged, `force_withdraw` from `CurrencyMigrated`, the expected
trim, `free_plus_reserved_unchanged`, and the stash's ledger, free, reserved, frozen, staking lock
and staking hold `before` (parent block) and `after` (inclusion block).

The summary, printed at the end and saved to `FILE.summary.json`:

- stash counts and errors
- fees charged, and the signer's free balance before and after
- total trimmed, against the dry run
- total issuance before and after
- the validators below `MinValidatorBond`, with the top-up each one needs

## What to check

- `errors: none`
- `fees charged: 0.0000`
- `total issuance: ... UNCHANGED`. The sweep moves no funds between accounts and mints nothing. A
  `payout_stakers` by anyone during the sweep mints rewards and changes it, so look at the blocks in
  between before treating a change as a sweep problem.
- the trimmed total matches the dry run, unless stakes changed in between
- the list of validators below the minimum, which goes to those validators

## Validators below the minimum

A validator whose active bond is below `MinValidatorBond` (100 PDEX) stays a validator, but
`staking.validate` (a commission change, for example) and `staking.unbond` fail with
`InsufficientBond` until the bond is back at the minimum. After the sweep a trimmed stash has exactly
1 PDEX free. To top up, transfer the shortfall shown in the summary plus about 1 PDEX for fees to the
stash, then sign `staking.bondExtra(shortfall)` with the stash.

## Anyone can migrate a stash

`staking.migrateCurrency(stash)` from any account, for example in Polkadot.js Apps under Developer >
Extrinsics. It is free when it succeeds and fails with `AlreadyMigrated` for a stash that has already
switched.

## Rehearsal

On a private copy of mainnet state at block 13,206,286 (10 Oct 2026), after 392 was enacted there
through the governance path:

- Dry run: 1,625 targets (75 validators), expected trim 1,135.1552 PDEX, 73 validators below the
  minimum afterwards.
- Before the sweep, `payout_stakers(v, 1839)` for two of those validators failed with
  `Staking.NotEnoughFunds`.
- The sweep migrated all 1,625 stashes in 33 waves, one block each, in 20.1 minutes: no errors, no
  resubmissions, 0 PDEX in fees, 1,135.1552 PDEX trimmed (equal to the dry run, stash by stash),
  total issuance unchanged.
- After it, the same two payouts succeeded (96 and 7 rewards paid). On five trimmed stashes, free
  balance plus hold equalled the free balance before.
- No call was dropped, so the resubmission path did not run.
