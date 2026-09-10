Plan: Optional credential redaction

Status: Implemented and verified on 2026-09-08. All schemas remain at 1.

Implemented extension: [child Push settings](PLAN_CHILD_SYNC_SETTINGS.md).
It adds an independent child stamp and shares the `export_settings_changed`
publication condition.

Use one app-wide switch on this machine. Default to On. Keep all schema
versions at 1. Add no migration or legacy execution path. The user will remove
local Mallard state and cloud test data before retesting.

1. Product behavior

   Replace `Always on` in Settings > Privacy with `Credential redaction`.
   Save changes immediately through the existing settings API.

   - On runs the current export redaction stages.
   - Off preserves transcripts, child sessions, sidecars, provider-state
     content, titles, and content-derived metadata. The export run performs
     no secret replacement or detector work.
   - Both keep session reconstruction, path mapping, source validation,
     bounded reads, hashes, and immutable publication.

   Show the mode in Push review. For Off, show:
   `Exported content can include API keys, tokens, and passwords.`
   Use the existing Push approval; add no switch confirmation dialog.

   Keep resource selection, credential-file exclusions, and existing content
   warning acknowledgements. Keep diagnostic and stderr sanitization active.
   Library and ordinary project-file payloads already travel without
   redaction; this change needs no new export work in those paths.

2. Settings and cache behavior

   Add `credential_redaction_enabled: bool` to Rust `AppSettings` and the
   TypeScript DTO. Use a serde field default of `true`, consistent with the
   existing partial-settings contract. A missing settings field means On.
   A missing wire stamp remains invalid; these are separate contracts.

   Load settings from the supplied repository once at review or Push start.
   This permits isolated test repositories to select Off without installing
   non-default process-wide atomics. Add no redaction atomic and change no
   startup fallback behavior. Use the loaded value for the whole operation.

   A redaction-only change has **no cache consequence**. Observation caches
   contain local-form evidence and adoption evidence for the cloud stamp.
   Neither depends on the outgoing switch. Keep the switch out of discovery
   requests and observation cache keys. Recompute the mode-dependent review
   decision from the saved setting and current manifest without a cold scan.

   In `src-tauri/src/app_settings.rs`, compare the previous and saved values
   of the four tuning fields. Call `reset_all_caches_and_purge_disk` only if
   at least one tuning field changed. Redaction-only and unchanged saves
   must preserve both memory and disk caches.

3. Put the mode on `RedactionRun`

   Extend the existing run in
   `crates/mallard-infrastructure/src/credential_redaction.rs`.
   Keep existing producer `&mut RedactionRun` parameters and
   `RedactedWireBytes`. Add no policy module or replacement payload type.
   Select the mode at operation or worker construction sites. Route export
   helpers that call the bare ruleset through the selected run, including
   capture and title generation.

   | Run operation | On | Off |
   | --- | --- | --- |
   | Redact | Existing behavior | Identity result; report no changes |
   | Verify credentials | Existing check | Success without scanning |
   | Certify document | Existing check and wrapper | Wrap unchanged payload without scanning |
   | Stamp | Current On stamp | Explicit Off stamp |

   Off returns before detector and cache work. It can still construct the
   existing immutable payload and its required content hash. Update the
   wrapper documentation: bytes have passed through the selected run;
   Off does not assert that they are secret-free. Keep private construction
   and immutable payload access.

   Bare `RedactionRuleset` methods stay always On. This preserves stderr
   scrubbing in `storage_catalog/adapters.rs`. Change export call sites,
   not the behavior of diagnostic callers.

   Keep the `wire_redaction` shape. On retains `mallard.redact.v1` and its
   current fingerprint. Off uses `mallard.redact.off.v1` and a fixed 64-zero
   fingerprint sentinel. This satisfies the existing 64-hex-character
   validator; it is not a derived policy hash. Accept only the supported
   codec/fingerprint pairs. Reject absent, unknown, or mismatched wire
   stamps. Publish the selected run's stamp.

4. Remove the two content blockers

   Remove the `detect_secret_material` call from generic provider-content
   capture. Do not replace it with a warning scan. The real redactor is the
   authority in On mode; Off preserves raw content. This also removes the
   lowercase allocation on each provider file. Retain the separate
   project-file warning and private-key exclusion checks that use the helper.
   An `access_token` key must not block provider capture when its value is
   raw or already a placeholder.

   Delete `reject_credential_shapes` and its calls from
   `crates/mallard-infrastructure/src/portable_resources/contracts.rs`.
   Manifest decoding validates structure independently of the local setting.
   Keep bounds, typed fields, IDs, digests, and path validation. Keep On
   redaction at payload and title producers. Do not redact structural
   identifiers or digest fields.

   An On machine must accept a valid Off manifest. Session content checks
   use the stored stamp, never the receiving machine's outgoing switch.

5. Review, publication, and Pull

   Add the selected mode to Push review and its approval/plan. At apply,
   compare it with the saved value once, before capture or publication.
   A difference requires a new review. The existing `manifest_sha256` pin
   already covers the stored stamp; add no second pin.
   Acquire the existing host operation lock before reading settings in
   apply, and before any settings write. Hold it through the operation.
   The lock uses a non-blocking attempt: a settings save during Push apply
   returns a busy error immediately. Do not queue or save the new value.
   Keep the switch at its saved value and show the error. The active Push
   completes with its approved run; the user can retry the setting change
   after Push ends. If the save finishes before apply starts, the mode pin
   check requires a new review when the values differ.

   The only new publication rule is for an otherwise current session whose
   stored mode differs from the selected mode. Show
   `Redaction setting changed` and allow the existing `Publish` action.
   Extend the canonical decision and action validation for this case.
   Keep the relation table and other decisions.

   Publish the complete session in the selected mode under its existing
   cloud ID. Derive hashes, wire tails, and child pins from the output and
   commit the accepted local/wire pair through the existing transaction.
   Publish the new stamp even when content bytes stay equal.

   Pull and first-contact adoption construct their run from the cloud row's
   stored stamp. Their purpose is to verify or compare that stored content;
   the local switch selects only the next outgoing publication. Show
   `Stored without credential redaction` for Off content in Pull review.

   `decode_codex_tree_member` currently creates an On run internally. Pass
   the stored-stamp run into that entry point. This is a required run
   parameter, not a separate policy argument on every builder.

   Mode changes do not change the tail codec grammar. On cannot remove
   secrets from older immutable revisions. Off cannot recover text already
   replaced with placeholders.

6. Performance and repeated On scans

   Off should reduce Push preparation CPU and temporary detector memory.
   It skips detector scans, replacements, cache hashing, and parsing used
   only for redaction checks. Required session parsing, path checks, payload
   hashing, transfer, and persistence remain. Existing return types can still
   require String or Value copies; Off is not a zero-copy design.

   Separate the first mode-change Push from later pushes. Selected sessions
   with a different stored mode require complete encoding and a new stamp,
   even without new messages. Changed payloads can require new uploads.
   Preserved secret text can also increase payload and retained storage size.
   A redaction-only save retains warm comparison caches. Pull and adoption
   work depend on the stored stamp, not the local outgoing switch.

   Before claiming a speedup, compare On and Off on the same synthetic
   fixtures in the same release build, with equal selections and cache state.
   Record preparation time, total time, detector work, peak memory, and bytes
   transferred. Report mode-change publication separately. No matched On/Off performance measurements are available.

   Keep the current On scan schedule in this change. Removing repeated
   Claude, sidecar, and provider-state scans is deferred to a measured
   follow-up. A mode-aware run does not itself remove those passes.

   Claude verifies changed JSONL lines, then parses and certifies the whole
   output again. Sidecars and provider state are scanned at capture and
   publication; provider-state publication uses an uncached ruleset call.
   Route that call through the selected run for correct mode behavior.
   Do not claim that this eliminates the second pass.

   Before removing a pass, measure these phases on fixed synthetic JSONL,
   JSON, plain-text, and binary fixtures in a release build. Record parsed
   bytes, detector work, elapsed time, and capture-to-publication cost.
   Use temporary homes. Compare repeated runs on the same fixtures and
   record cache state. Historical timings are not a new baseline.

   Prove that unchanged and changed content retain their checks, including
   plain-text routing. Keep payload hashing and path checks. Use full
   certification as a test oracle before removing a production pass.
   Follow the proof approach in
   `docs/PLAN_REMOVE_CODEX_REDACTION_RESCANS.md`; the Codex proof does not
   automatically cover other producers. No On-mode speedup is claimed here.

7. Implementation and verification

   Implement settings and run mode first. Remove the capture and manifest
   blockers. Connect operation constructors and stamps, add the review pin
   and current-session publication rule, then wire the UI. Update Rust DTOs
   and `src/types.ts` together through the existing command/API boundaries.
   Update README privacy claims. Keep the observation-cache format.

   Add four focused checks to existing tests:

   - Run semantics: On matches existing fixtures. Off preserves content.
     Cache-eligible inputs produce zero detector-cache lookups, insertions,
     hits, and hashed bytes; use an On control with nonzero work. Bare
     ruleset calls remain On. Validate stamps and rejection cases.
   - Settings and review: missing field defaults On; both values save and
     load; UI save failure retains the saved value. Redaction-only and
     unchanged saves preserve caches; tuning changes still purge.
     A changed approved mode rejects apply. A save attempted during apply
     returns busy, leaves the saved value unchanged, and cannot change the
     active run. A later retry can save the new value.
   - Content boundaries: raw marker-bearing provider state and metadata
     publish Off and can be read by an On machine. Placeholder-valued
     markers do not fail capture. Reuse structural rejection tests.
   - Session lifecycle: extend Codex and Claude fixtures for both modes,
     child/sidecar content, opposite-setting Pull/adoption, and mode-only
     republication in both directions. Reuse existing conflict, checkpoint,
     integrity, and restore coverage.

   Run focused checks during implementation, then:

   ```sh
   npm run build
   npm run test:frontend-integration
   cargo test --workspace --lib
   cargo test -p mallard-infrastructure --doc
   ```

   Verify Settings and review in the complete Tauri app with temporary homes
   and capture screenshots. No real provider data or credentials enter tests.

8. Verification results — 2026-09-08

   - `npm run build`: passed. Vite reports the existing large-chunk warning.
   - `npm run test:frontend-integration`: 294 passed.
   - `cargo test --workspace --lib`: 928 passed; 10 existing tests ignored.
   - `cargo test -p mallard-infrastructure --doc`: 1 passed.
   - `git diff --check`: passed.

   Synthetic Codex and Claude fixtures cover On → Off → On publication under
   one cloud ID, parent and child content, Claude sidecars and provider state,
   metadata, a stale mode approval, opposite-setting Pull, and first-contact
   adoption. Old immutable Off content remains readable. Source files stay
   unchanged. Off run tests record zero detector-cache work.

   Native Tauri checks used temporary Mallard and provider homes. They verified
   default On, saved Off, a busy save that retained Off, a successful retry to
   On, the Off Push warning, the visible mode-change reason, a successful
   mode-only Push, and the stored-Off Pull notice. Save errors appear beside
   the affected setting. Push row filters now accept a current session when
   the canonical decision permits publication.

   Native screenshots from this verification:

   - [Settings Off](/private/tmp/mallard-redaction-work/settings-off.png)
   - [Busy save retains Off](/private/tmp/mallard-redaction-work/settings-busy.png)
   - [Off Push review](/private/tmp/mallard-redaction-work/push-off.png)
   - [Stored-Off Pull notice](/private/tmp/mallard-redaction-work/pull-off.png)

   Repeated On scans remain deferred as specified in section 6. No measured
   speedup is claimed. The user’s local state and cloud data were not reset.
