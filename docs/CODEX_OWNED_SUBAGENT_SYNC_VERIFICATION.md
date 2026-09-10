# Owned Codex child sync: verification

Date: 2026-09-08. Local-folder storage. Release build on macOS arm64.

The implementation keeps schema 1. It adds no migration, compatibility reader,
legacy execution path, stored omission mode, or portable Mallard boundary.

## Functional checks

- Ownership excludes inherited foreign children and includes descendants.
- Both native activity forms retain the same creation ID. Unsafe IDs, conflicting metadata, and changed ownership fail validation.
- A known unsupported owned child selects main-only scope. All children are excluded. Missing required evidence blocks the task.
- Push accepts known-member appends before capture. Changed membership fails only the affected task. An all-failed batch retains its existing HEAD.
- Child-only changes affect Compare. Mixed local and cloud work is divergent. New descendants after Pull are discovered.
- Pull validates the declared tree, restores separate native files, and stores local/normalized pairs. Keep both retains the original child files and allocates different IDs.
- Failure injection covers a second provider-file write and each local metadata document. Completed writes remain. The task reports failure.
- The Push review icon has a keyboard focus target, an accessible label, and a tooltip. It is absent from browse and Pull views in the frontend tests.

The native Codex 0.152.0 test passed with temporary homes and a deterministic
loopback model. Push and Pull transferred a parent and a paginated child with
copied history. A fresh Codex process resumed the imported parent and sent a
follow-up to the existing child. Both files grew. The source tree did not change.
The test made eight mock model requests and used no service credentials.

The shared-home omission test also passed. Pull restored only the main when the
owned child lacked its boundary. A cold native resume could not control the
omitted source child. The original source files stayed unchanged.

The approved isolated Tauri test passed. The Push icon tooltip is fully visible
and keyboard accessible. A main-only Push completed and the task became synced.
The test used synthetic homes and local-folder storage. Screenshot evidence is
stored outside the repository as `mallard-owned-child-push-review.png`.

## Redundant-work audit

The audit removed repeated work in child discovery, Push, and Pull:

- Merge installed children and activity references before looking up files or reading owner metadata.
- Parse authoritative child metadata once per header read.
- Check Push cutoffs against captured bytes. Included child metadata uses the final source stamp; separate header witnesses cover foreign and omitted children.
- Retain main creation records from its verified wire graph. Use each child's graph parse for creation records, native ordinals, ownership boundary, and normalized content state.
- Use the builder's verified output and local content state. Carry the existing prepared child vector through Apply as immutable verification results.
- Match each child result to the installation map, accepted pair, project, and target bytes before staging. The commit path still checks each staged file's SHA-256 digest.
- Keep activity-representation validation in the required decode parse. Do not run another JSON pass for that check.

No persisted fields or schema changes were added by this audit. The obsolete
full-analysis child validation path was removed. The existing main verification
mechanism now also binds child output bytes to their installation.

Work-count tests require exactly two semantic analyses per prepared child:
one wire graph analysis and one local output analysis. Tamper tests reject
changed content, owner, ordinal, boundary, accepted hashes, target identity,
project root, cutoff, missing children, and altered staged bytes.

The work-count tests also verify one filename traversal for a cold tree, zero
child body reads or graph resolutions for an unchanged warm tree, and one parsed
record with no sibling body reads after a single-child append. Refresh requests
cloud metadata; it does not download cloud transcript objects.

## Measured performance

Each rollout contains 128 added messages with 2,048 text characters per message,
plus metadata and activity records. The wide case has 16 direct children. The
nested case has eight descendants. Refresh uses 15 samples per case; Apply uses
10. Values are **median / p95 milliseconds**. With 10 samples, p95 is the largest
sample. These are local measurements, not latency guarantees.

Current release build, after the redundant-work audit:

| Operation | Main only | 16 children | Eight nested children |
| --- | ---: | ---: | ---: |
| Cold Refresh | 20.8 / 22.9 | 58.3 / 68.2 | 39.8 / 45.9 |
| Warm Refresh | 8.6 / 10.2 | 11.3 / 12.9 | 9.2 / 10.7 |
| Append + Push Review | 48.8 / 53.0 | 46.3 / 52.6 | 38.0 / 51.5 |
| Push Apply: existing task update | 105.6 / 112.8 | 258.7 / 269.4 | 180.7 / 184.3 |
| Pull Apply: existing task update | 336.8 / 356.9 | 628.5 / 679.5 | 469.5 / 530.2 |
| Pull update excluding restart-offer check | 120.6 / 128.1 | 390.6 / 419.5 | 260.8 / 268.7 |
| Pull Apply: fresh destination | 305.5 / 353.5 | 617.2 / 733.3 | 467.4 / 488.6 |

The update comparison appends to one rollout, pushes that update, then pulls it
into an existing destination. Review is outside both timers. The earlier
673–691 ms Pull figure used fresh destinations and included the restart-offer
check. A fresh destination alone did not explain the Push/Pull difference.

Pull performs the existing Codex restart-offer check after it commits the files.
It queries the running app through `/usr/bin/osascript`. The application-level
benchmark includes this production path. Its median was about 210–245 ms across
these runs. Push has no equivalent check. The exclusion row subtracts the
measured duration from each sample before computing percentiles. The query still
runs in the application; it has not been moved off the completion path.

### Before and after this audit

The pre-audit release test executable was retained and rerun before the rebuilt
executable. Both ran the same fixtures on the same host. This comparison measures
the audit changes; it is not a baseline from before subagent support.

| Median ms | Before audit | After audit |
| --- | ---: | ---: |
| Main-only Pull update excluding restart offer | 121.2 | 120.6 |
| 16-child preparation | 189.9 | 135.0 |
| 16-child staging and commit | 179.9 | 146.4 |
| 16-child Pull update excluding restart offer | 476.9 | 390.6 |
| 16-child Pull update, full wall time | 724.5 | 628.5 |
| Eight-child Pull update excluding restart offer | 287.7 | 260.8 |

The 16-child Pull cost excluding the restart offer fell by about 18%. The
remaining child work includes native reconstruction, wire/output verification,
file staging, durable writes, and local state commits. Stage medians are measured
separately and do not sum exactly to the median wall time.

The exact pre-feature baseline, large legacy-fork header sets, remote-storage
latency, and larger payloads still need separate release measurements. Peak
resident memory was not remeasured for this audit. Immutable payload references
share their bytes; retaining a verification result does not copy the rollout.

## Reproduce

```sh
cargo test --workspace --all-targets
npm run build
npm run test:frontend-integration
node scripts/run-codex-owned-sync-native.mjs
node scripts/run-codex-owned-sync-native.mjs --shared-home-omission
cargo test --release -p mallard --lib benchmark_owned_subagent_sync -- --ignored --nocapture --test-threads=1
```

## Remaining validation limits

- The native client did not expose `spawn_agent` inside a child in the isolated test configuration. Nested creation and native cold resume of a grandchild are not signed off. Nested transport, comparison, new membership, and Keep both pass Rust fixtures.
- The exact pre-change release baseline and the legacy-fork performance matrix remain release gates.
- The original mixed-provider POC pins Claude 2.1.260; the installed Claude 2.1.263 fails its version gate. That POC was not counted as a current pass.
