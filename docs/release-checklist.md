# Release checklist

Every release of `polkadex-node` follows this list, top to bottom. Nothing is skipped because it is small. If a step cannot be done, the release waits.

Roles: **Dev** writes the code and opens PRs. **Reviewers** are two people who are not the author. **Release manager** signs and publishes. **Ops** runs the team's own nodes. One person may hold more than one role, but the author of a change never reviews or signs it.

## 1. Before the tag

- [ ] Every change is on `testnet` through a PR with two approvals. No direct pushes, no exceptions.
- [ ] `testnet` has run with these changes on the testnet chain for at least 3 days with no incident.
- [ ] The same changes are merged to `mainnet` through a PR with two approvals.
- [ ] CI is green on the `mainnet` head.
- [ ] If the runtime changed: `spec_version` is bumped. If any call or transaction extension changed shape: `transaction_version` is bumped too.
- [ ] If the runtime changed: try-runtime has been run against a live mainnet snapshot, unfiltered, and the log is attached to the PR. Every migration in the tuple is listed in the PR with what it touches and whether it is guarded.
- [ ] Release notes draft written: what changed, what operators must do, whether validators need to rotate keys.
- [ ] Release manager and Ops have agreed a date and a window.

## 2. Tag

- [ ] Tag is created on the `mainnet` branch only, named `vX.Y.Z`, annotated with the release title.
- [ ] Tag commit hash is recorded in the release notes.

## 3. Build and verify

- [ ] The release workflow ran on the tag and produced: binary, `.deb`, `.rpm`, `customSpecRaw.json`, `SHA256SUMS`, as a draft release.
- [ ] Release manager downloads all artefacts and runs `sha256sum -c SHA256SUMS`.
- [ ] Release manager confirms the workflow run used the pinned toolchain and pinned actions (no warnings about floating versions in the run log).
- [ ] Dev installs the `.deb` on a clean Ubuntu 22.04 and the `.rpm` on a clean Rocky 9, starts the service, confirms it reaches peers and reports genesis `0x3920bcb4960a1eef5580cd5367ff3f430eef052774f78468852f7b9cb39f8a3c`. Logs attached to the draft release.

## 4. Sign and publish

- [ ] Release manager signs: `gpg --detach-sign --armor SHA256SUMS`, uploads `SHA256SUMS.asc` to the draft.
- [ ] Release manager checks the release notes render correctly and include the verify instructions.
- [ ] Release manager publishes the release. Nobody else has the signing key.

## 5. The team's own nodes first

- [ ] Ops upgrades the public RPC node(s) using the package. Confirm they are back at head and serving.
- [ ] Ops upgrades the bootnode(s).
- [ ] Ops confirms telemetry shows the new version on the team's nodes.

## 6. Validators

- [ ] Announcement to validators with: the release link, the verify steps, the window by which the client upgrade must be done, and the migration doc link for anyone still on the old setup.
- [ ] If a runtime upgrade follows: the enactment date, and the instruction to rotate session keys after enactment if the release notes say so.
- [ ] A named person watches telemetry for validators dropping off during the window and contacts them.

## 7. Runtime upgrade (only when the release carries one)

- [ ] Client upgrade window has closed and the large majority of validators report the new version.
- [ ] The `set_code` proposal goes through governance as documented for this runtime. No Sudo exists.
- [ ] At enactment: Ops watches block production and finality for two sessions. If either stalls, escalate immediately.
- [ ] After enactment: validators rotate keys if required. Ops confirms the new session key set on the team's validators.

## 8. After

- [ ] Release notes updated with anything learned.
- [ ] Any follow-up items filed as issues with an owner.
- [ ] This checklist updated if a step was missing or wrong.

## Rollback

A client release that misbehaves: validators reinstall the previous package version. Packages for the previous release stay attached to their GitHub release, never deleted.

A runtime upgrade that misbehaves cannot be rolled back by validators. It needs a new runtime upgrade through governance. This is why section 1 is not optional.
