# Release checklist

Every release of `polkadex-node` follows this list, top to bottom. Nothing is skipped because it is small. If a step cannot be done, the release waits.

Roles: **Dev** writes the code and opens PRs. **Reviewers** are two people who are not the author. **Release manager** signs and publishes. **Ops** runs the team's own nodes. One person may hold more than one role, but the author of a change never reviews or signs it.

## 1. Before the tag

- [ ] Every change is on `testnet` through a PR with two approvals. No direct pushes, no exceptions.
- [ ] `testnet` has run with these changes on the testnet chain for at least 3 days with no incident. No incident means: blocks produced continuously, finality lag under 10 blocks, no drop of more than 10 percent in the active validator count, no error-level log lines on the team's own nodes. Ops calls it, in writing, before the mainnet PR is opened.
- [ ] The same changes are merged to `mainnet` through a PR with two approvals.
- [ ] The CI workflow has run on the exact commit being tagged and every job is green. (Prerequisite: `ci.yml` must trigger on `testnet` and `mainnet` and on every push to a PR, and build only crates that exist. Until that repair lands this item cannot be ticked and the release waits.)
- [ ] The pin check job passed: every `uses:` in every workflow is a 40-character commit hash. Actions emits no warning for floating tags, so this is a CI step, not a human check.
- [ ] If the runtime changed: `spec_version` is bumped. If any call or transaction extension changed shape: `transaction_version` is bumped too.
- [ ] If the runtime changed: try-runtime has been run against a live mainnet snapshot, unfiltered, and the log is attached to the PR. Every migration in the tuple is listed in the PR with what it touches and whether it is guarded.
- [ ] Release notes draft written: what changed, what operators must do, whether validators need to rotate keys.
- [ ] Release manager and Ops have agreed a date and a window.

## 2. Tag

- [ ] Tag is created on the `mainnet` branch only, named `vX.Y.Z`, annotated with the release title. Enforced, not assumed: the release workflow's first step asserts `git merge-base --is-ancestor "$GITHUB_SHA" origin/mainnet` and fails otherwise, since Actions cannot filter tags by branch.
- [ ] Tag commit hash is recorded in the release notes.

## 3. Build and verify

- [ ] The release workflow ran on the tag and produced: binary, `.deb`, `.rpm`, `customSpecRaw.json`, `SHA256SUMS`, as a draft release.
- [ ] Release manager downloads all artefacts and runs `sha256sum -c SHA256SUMS`.
- [ ] Release manager confirms the workflow run used the toolchain from `rust-toolchain.toml` and that the pin check job passed.
- [ ] Dev installs the `.deb` on a clean Ubuntu 22.04 and the `.rpm` on a clean Rocky 9, starts the service, confirms it reaches peers, reports genesis `0x3920bcb4960a1eef5580cd5367ff3f430eef052774f78468852f7b9cb39f8a3c`, and appears on telemetry under the `NODE_NAME` set in `node.env` on both distributions. An empty or default name on telemetry is a failure. Then upgrades the package over itself and confirms `systemctl is-active polkadex-node` afterwards and the new version in the logs; a successful install that leaves the service stopped is a failure. Logs attached to the draft release.

## 4. Sign and publish

- [ ] Release manager signs: `gpg --detach-sign --armor SHA256SUMS`, uploads `SHA256SUMS.asc` to the draft.
- [ ] Release manager checks the release notes render correctly and include the verify instructions.
- [ ] Release manager publishes the release. The signing key is held by the release manager only. Its offline backup and revocation certificate locations are recorded in the private ops document, not here.

## 5. The team's own nodes first

- [ ] Ops upgrades the public RPC node(s) using the package. Confirm they are back at head and serving.
- [ ] Ops upgrades the bootnode(s).
- [ ] Ops confirms telemetry shows the new version on the team's nodes.
- [ ] Stop rule: if any of the team's nodes does not return to head within 30 minutes of its upgrade, the release halts here. The release manager decides whether to continue or roll the team's nodes back. Validators are not contacted until this section is complete.

## 6. Validators

- [ ] Announcement to validators with: the release link, the verify steps, the window by which the client upgrade must be done, and the migration doc link for anyone still on the old setup.
- [ ] If a runtime upgrade follows: the enactment date, and the instruction to rotate session keys after enactment if the release notes say so.
- [ ] A named person watches telemetry for validators dropping off during the window and contacts them.
- [ ] Stop rule: if validators holding more than 10 percent of active stake drop off during the window and do not return within an hour, the release manager pauses the announcement and the runtime upgrade date until the cause is known.
- [ ] External communications: a short notice to users and exchanges with the date and what changes for them, and a prepared statement for the case where something goes wrong. Ops owns the wording.

## 7. Runtime upgrade (only when the release carries one)

- [ ] The client upgrade is not a condition for enactment. Confirmed for this release, not assumed, by three checks that Ops runs and records:
  1. Host functions: decompress the new runtime (`:code` from the chain or the built `.compact.compressed.wasm`, strip the 8-byte magic, `zstd -d`), list its imports (`wasm-objdump -x -j Import` or a WASM import-section parser), and diff against the live runtime's imports. Every new import must be a host function the oldest supported client provides. The oldest supported client for this release is v6.2.0 on polkadot-sdk 1.1.0.
  2. Runtime APIs: diff `state_getRuntimeVersion.apis` between live and new. Any version change on an API the client calls (Core, BlockBuilder, TaggedTransactionQueue, BabeApi, GrandpaApi, SessionKeys, OffchainWorkerApi, TransactionPaymentApi) must be shown compatible with the oldest supported client.
  3. The real thing: run the oldest supported client binary (the `polkadex/mainnet:v6.2.0` image) against a silo that has the new runtime applied, and confirm it imports and authors blocks.
  Validators on the old client keep producing and finalising after enactment and only miss BEEFY participation, which nothing consumes yet.
- [ ] Validators holding at least two thirds of active stake are known to be online and following the chain in the day before enactment, by telemetry, peer version, or direct contact.
- [ ] BEEFY stays unstarted at enactment. Observed, not assumed: `beefy.genesisBlock()` reads `None` on testnet after the soak and on mainnet after enactment. If it ever reads `Some`, the warning below applies. It is started later through `beefy.set_new_genesis` by governance, only once validators holding more than two thirds of the active set have real BEEFY keys and a BEEFY-capable client. Starting it earlier leaves it permanently stuck at the first session that cannot be finalised.
- [ ] The governance call that chills validators still on placeholder keys is designed and merged in the spec after this one, with an owner and an issue, before a BEEFY start date is announced. It is a dependency of starting BEEFY, not an afterthought.
- [ ] Reaching that threshold is done by shrinking the set, not by persuasion alone: after a published deadline, validators whose BEEFY session key is still the placeholder are chilled by a governance call (runtime item for the following spec). Chill in rounds, never below a set where the three largest operators together hold under a third of remaining stake. If the upgraded set is too small for that, extend the deadline rather than chill deeper. Chilled validators rejoin by rotating keys and calling validate.
- [ ] The `set_code` proposal goes through governance as documented for this runtime. No Sudo exists, and the runtime test asserting that no pallet named `Sudo` is in the metadata passed on the tagged commit.
- [ ] At enactment: Ops watches block production and finality for two sessions. If either stalls, escalate immediately.
- [ ] After enactment: validators rotate keys if required. Ops confirms the new session key set on the team's validators.

## 8. After

- [ ] Release notes updated with anything learned.
- [ ] Any follow-up items filed as issues with an owner.
- [ ] This checklist updated if a step was missing or wrong.

## Rollback

A client release that misbehaves: validators reinstall the previous package version. Packages for the previous release stay attached to their GitHub release, never deleted.

A runtime upgrade that misbehaves cannot be rolled back by validators. It needs a new runtime upgrade through governance. This is why section 1 is not optional.
