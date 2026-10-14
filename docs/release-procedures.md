# Release procedures

How the steps in `release-checklist.md` are carried out. The checklist says what must be true; this file says how to establish it. Numbered to match the references in the checklist.

## Procedure 1: the testnet soak

Three days on the testnet chain with no incident. No incident means all of:

- blocks produced continuously, no gap longer than one slot, except a same-slot race (below)
- finality lag under 10 blocks
- active validator count not down by more than 10 percent
- no error-level log lines on the team's own nodes, except the exempt lines below

**Same-slot race.** Two validators author a block for the same slot, both blocks appear in the nodes' logs, and one is dropped when the heavier chain wins. This shows as a gap of one or two slots even when everything works, and it happens on mainnet too. A gap is a same-slot race, not an incident, when both blocks are in the logs, the gap is at most 3 slots and finality lag stays under 10 blocks. Ops lists every race with its log evidence. Any other gap longer than one slot is an incident.

**Exempt error lines.**

- The `Corrupted state at` line at the upgrade block (procedure 6, point 2).
- Lines caused by a recorded test action, such as a transaction sent on purpose during the soak. Ops records the action, its time and the exact lines it caused, and says why they do not apply on mainnet or how the cause is fixed.

Ops watches for the three days and records the result in writing before the mainnet PR is opened, against these criteria and with the raw evidence attached. A single incident restarts the three days.

## Procedure 2: the tag branch gate

GitHub Actions cannot restrict a tag trigger to one branch, so the release workflow enforces it. Its first step is:

```
git fetch origin mainnet
git merge-base --is-ancestor "$GITHUB_SHA" origin/mainnet || { echo "tag is not on mainnet"; exit 1; }
```

A tag on any other branch fails the build before anything is produced.

## Procedure 3: the install test

On a clean Ubuntu 22.04 and a clean Rocky 9 machine:

1. Install the package (`dpkg -i` / `dnf install`). Set `NODE_NAME` in `/etc/polkadex/node.env`, start the service.
2. Confirm the node reaches peers and reports genesis `0x3920bcb4960a1eef5580cd5367ff3f430eef052774f78468852f7b9cb39f8a3c`.
3. Confirm the node appears on telemetry under the `NODE_NAME` that was set, on both distributions. An empty or default name means the `${NODE_NAME}` expansion in the unit failed.
4. Install the same package again over the running service. Confirm `systemctl is-active polkadex-node` afterwards.
5. Upgrade to the new package over the running service. Confirm `systemctl is-active polkadex-node` afterwards and the new version in the logs. A successful install that leaves the service stopped is a failure; on rpm the old package's `%preun` runs after the new `%post`, which is where an unconditional stop shows.
6. Purge. Confirm `/etc/polkadex` is removed and `/var/lib/polkadex` is kept.
7. For a release validators migrate to: one full run of `migrate-to-packaged-node.md` on a box set up the old way, confirming it resumes at the same block with the same keystore.

Attach all logs to the draft release.

## Procedure 4: client compatibility for a runtime upgrade

The question is whether validators still on an older client can execute the new runtime. Three checks, in order. Record the results with the release.

1. **Host functions.** Get the new runtime WASM: `:code` from the chain that has it, or the built `.compact.compressed.wasm`. Strip the 8-byte magic prefix and decompress with `zstd -d`. List the import section (`wasm-objdump -x -j Import`, or any WASM import-section parser). Do the same for the live runtime. Every import that is new must be a host function the oldest supported client provides. State which client that is; for the 392 release it is v6.2.0 on polkadot-sdk 1.1.0 (September 2023).
2. **Runtime APIs.** Compare `state_getRuntimeVersion.apis` between live and new. Any version change on an API the client itself calls (Core, BlockBuilder, TaggedTransactionQueue, BabeApi, GrandpaApi, SessionKeys, OffchainWorkerApi, TransactionPaymentApi, AuthorityDiscoveryApi) must be shown compatible with the oldest supported client, by reading what changed in that version. New APIs that the client does not call are irrelevant.
3. **The real thing.** Run the oldest supported client binary, the `polkadex/mainnet:v6.2.0` image, against a silo that has the new runtime applied. Confirm it imports blocks and authors them. This is the only check that catches what nobody thought to measure.

If any check fails, the client upgrade becomes a condition for enactment and section 7 waits until more than two thirds of the active validators, counted by validator and not by stake, are on the new client. GRANDPA gives every validator in the set one vote regardless of stake, so a finality quorum is 134 of 200 validators, and two large operators cannot stand in for it.

## Procedure 5: BEEFY and the validator set

BEEFY must not start until more than two thirds of the active validators, counted by validator, have real BEEFY keys and a BEEFY-capable client. BEEFY counts authorities, not stake: with N authorities it needs `N - floor((N-1)/3)` signatures, which is 134 of 200. Started earlier, it sticks permanently at the first session it cannot finalise.

- At enactment the runtime sets no BEEFY genesis. Confirm by reading `beefy.genesisBlock()`: it must be `None` on testnet after the soak and on mainnet after enactment. If it ever reads `Some` before the threshold is met, treat it as an incident.
- Track the threshold from chain state: a validator's BEEFY session key that is all zeros is the placeholder from the migration. Count validators with real keys, and check `beefy.authorities()` has the same length as `session.validators()` after enactment; a shorter list means the BEEFY authority bound is truncating the set and must be fixed before any start.
- Reaching the threshold is done by shrinking the set, not persuasion alone. After a published deadline, validators still on placeholder keys are chilled by a governance call. That call is a runtime item for the spec after the upgrade and needs an owner and an issue before any BEEFY start date is announced.
- Chill in rounds. Never take the set below a size where the three largest operators together run a third or more of the remaining validators, since that many nodes going down together would stop finality. If the upgraded set is too small for that, extend the deadline rather than chill deeper.
- Chilled validators rejoin by rotating keys and calling `validate`. Do not force-unstake; that starts a 28-era unbonding clock for no benefit.
- When the threshold holds for a few eras, governance starts BEEFY with `beefy.set_new_genesis` pointing a little ahead of the current block.

## Procedure 6: notes for the 392 upgrade

Points 1 to 3 go into the release notes and the validator announcement (section 6). Point 4 is for whoever uploads the runtime. Point 5 is for the team, right after enactment.

1. **Session keys.** From 392 on, session keys belong to the account that signs `session.setKeys`. `ValidatorIdOf` is `ConvertInto`, because `StashOf`, which mapped a controller to its stash, no longer exists in the SDK. To rotate, run `author_rotateKeys` on the node, then sign `session.setKeys` with the stash account. Pass the `author_rotateKeys` output as `keys` and `0x` as `proof`. A `setKeys` signed by a controller account succeeds but stores the keys on the controller, so nothing changes for the validator. Many validators have a separate controller, so the announcement must say this plainly.
   - New keys become active two session boundaries after `setKeys`, 4 to 8 hours on mainnet. Keep the old keys on the node until then (see the C7 steps in security-audit-fixes.md).
   - Every validator rotates once after the upgrade enacts, to replace the placeholder BEEFY and mixnet keys from the migration (procedure 5). Keys set before the upgrade keep working until the rotated keys are active, so the rotation causes no gap.
   - In Polkadot.js Apps, the Session Key button under Staking > Account Actions signs with the controller. With a separate controller, use Developer > Extrinsics and select the stash as the signing account.
   - A validator who already signed with the controller runs `author_rotateKeys` again and signs with the stash. The same keys sent from the stash fail with `DuplicatedKey`, because the controller now holds them.
2. **One error line at the upgrade block.** In the first block that runs 392, each node logs one error line that starts with `Corrupted state at`, followed by the `System::Events` storage key as a byte list, `[38, 170, 57, 78, ...]`. The new runtime reads the events that the old runtime wrote in that block, and the event format changed. It is harmless and does not repeat, so it does not count against the soak in procedure 1.
3. **Stakers.** From 392, staked PDEX is kept on hold instead of under a lock. Each account switches the first time its stake changes on 392, usually when a reward is paid and restaked, or when anyone calls `staking.migrateCurrency` for it. Wallets then show the bonded amount as reserved instead of locked. The total balance does not change, and unbonding works as before. A hold must leave the existential deposit, 1 PDEX, free, so a stash whose free balance minus 1 PDEX is below its bond keeps only what it can hold bonded; the rest stays in the stash, unbonded. The team switches those stashes right after enactment (point 5). A validator left below the 100 PDEX minimum bond tops up as point 5 says.
4. **Preimage cost.** Uploading the `set_code` preimage costs a fee plus a reserved deposit: 126.8988 PDEX in fees and a 13.6887 PDEX deposit, measured on the soak chain, a copy of mainnet at block 13,206,286, with this runtime (the preimage, the encoded `set_code` call, is 1,268,873 bytes). The uploading account must hold both before the upload.
5. **Staking sweep, right after enactment.** A stash whose free balance minus 1 PDEX is below its bond cannot move its whole bond onto a hold, so any update of its staking ledger fails with `Staking.NotEnoughFunds`. `payout_stakers` updates the validator's ledger before it pays anyone, so the payout of such a validator fails for the validator and all of its nominators. On mainnet state of 10 Oct 2026 that is 1,625 of 3,410 stashes, 75 of them validators. `staking.migrateCurrency(stash)` fixes a stash: it replaces the lock with a hold and lowers the bond to what the stash can hold (`Staking.CurrencyMigrated`, `force_withdraw`). Nothing leaves the stash. The call is permissionless, and its fee is refunded when it succeeds.
   - **Who:** the team, from an ops machine, with `tools/staking-migrate-currency/migrate_currency.py` (its README has the details). It is signed by a dedicated ops account that holds about 10 PDEX and nothing else. The call is permissionless, so the account needs no role; its balance only pays for calls that fail.
   - **When:** right after enactment, before the first payouts. Until it has run, payouts for those validators fail.
   - **How:** against the team's own synced node, a dry run first, then the sweep:
     ```
     python3 tools/staking-migrate-currency/migrate_currency.py --rpc ws://127.0.0.1:9944 --dry-run > dryrun.txt
     python3 tools/staking-migrate-currency/migrate_currency.py --rpc ws://127.0.0.1:9944 \
       --signer-file <file with the ops account's secret> --payee --out sweep.jsonl > sweep.log 2>&1
     ```
     The sweep sends only `Staking.migrate_currency`, 50 calls a block, and waits for each wave to be finalized; `--payee` then runs the payee pass below, which sends only `Staking.update_payee`. On the soak chain, a copy of mainnet state, the sweep migrated 1,625 stashes in 33 waves in 20 minutes, with no errors, 0 PDEX in fees and total issuance unchanged.
   - **Payee pass (`--payee`):** 29 stashes, 17 nominators and no validators, still have the deprecated `Controller` reward destination. They are still paid, to their controller, but every payout that includes one makes each node log an error line, `runtime::defensive` "Paying out controller as reward destination which is deprecated"; 11 of the 200 exposures of era 1839 include one. The payee pass calls `staking.updatePayee(controller)` for each of them, which turns `Controller` into `Account(<the same controller>)`: rewards go to the same account as before, and the error line stops. The call is permissionless and free when it succeeds. On the soak chain it changed all 29 in one block with 0 fees, and an era-1839 payout that included one of them paid that nominator's controller, the same account, with no error line on any node.
   - **Check** the summary at the end of `sweep.log`: no errors; fees charged 0; total issuance unchanged; the trimmed total equal to the dry run's (1,135.16 PDEX on 10 Oct state, at most 74.81 PDEX on one stash); the list of validators below `MinValidatorBond`; and for the payee pass, no errors, 0 fees, every destination unchanged and no `Controller` payees left. Keep `sweep.jsonl` and its `.summary.json` with the release record. Running the script again is safe; it skips stashes that have switched.
   - **Validators below the minimum** (100 PDEX; 73 on 10 Oct state, 71 of them short by at most 1 PDEX and 2 by about 65.5 PDEX) stay validators, but `staking.validate` (a commission change, for example) and `staking.unbond` fail with `InsufficientBond` until they top up. After the sweep their stash has exactly 1 PDEX free. They transfer the shortfall shown in the summary plus about 1 PDEX for fees to the stash, then sign `staking.bondExtra(shortfall)` with the stash.
