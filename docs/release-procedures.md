# Release procedures

How the steps in `release-checklist.md` are carried out. The checklist says what must be true; this file says how to establish it. Numbered to match the references in the checklist.

## Procedure 1: the testnet soak

Three days on the testnet chain with no incident. No incident means all of:

- blocks produced continuously, no gap longer than one slot
- finality lag under 10 blocks
- active validator count not down by more than 10 percent
- no error-level log lines on the team's own nodes

Ops watches for the three days and records the result in writing before the mainnet PR is opened. A single miss restarts the three days.

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

Points 1 and 2 go into the release notes and the validator announcement (section 6). Point 3 is for whoever uploads the runtime.

1. **Session keys.** From 392 on, session keys belong to the account that signs `session.setKeys`. `ValidatorIdOf` is `ConvertInto`, because `StashOf`, which mapped a controller to its stash, no longer exists in the SDK. To rotate, run `author_rotateKeys` on the node, then sign `session.setKeys` with the stash account. Pass the `author_rotateKeys` output as `keys` and `0x` as `proof`. A `setKeys` signed by a controller account succeeds but stores the keys on the controller, so nothing changes for the validator. Many validators have a separate controller, so the announcement must say this plainly.
   - Keys set before the upgrade keep working. Rotating after it replaces the placeholder BEEFY and mixnet keys from the migration (procedure 5).
   - In Polkadot.js Apps, the Session Key button under Staking > Account Actions signs with the controller. With a separate controller, use Developer > Extrinsics and select the stash as the signing account.
   - A validator who already signed with the controller runs `author_rotateKeys` again and signs with the stash. The same keys sent from the stash fail with `DuplicatedKey`, because the controller now holds them.
2. **One error line at the upgrade block.** In the first block that runs 392, each node logs one error line that starts with `Corrupted state at`, followed by the `System::Events` storage key as a byte list, `[38, 170, 57, 78, ...]`. The new runtime reads the events that the old runtime wrote in that block, and the event format changed. It is harmless and does not repeat, so it does not count against the soak in procedure 1.
3. **Preimage cost.** Uploading the `set_code` preimage costs a fee plus a reserved deposit: [PREIMAGE FEE AND DEPOSIT]. The uploading account must hold both before the upload.
