# Push after a Claude desktop import, and the first Push of a kept copy

- Status: implemented on 2026-09-29, on the owner's word ("Implement, don't
  worry about the child"). Not committed. Sections 1 to 11 are the plan as
  it was approved, with three corrections that section 12 names. Section 12
  says what was built and measured. Section 13 is the import of a session
  that Pull installed, implemented the same day on the owner's word
  ("Implement the cutoff fix first"). Section 14 is the Pull on the
  computer that made the children of a session, and the rows that an
  import rewrote, implemented the same day on the owner's word ("Implement
  all three"). Not committed.
- Date: 2026-09-29
- Owner's words: "the rewrite must be invisible to sync comparison"
  (2026-08-20, `PLAN_CLAUDE_DESKTOP_IMPORT_TOLERANCE.md`); "I expect mallard
  user to not change project directory for a session" (2026-09-29, on the
  accepted risk of decision 6); "Do not commit or push unless the owner
  asks."
- Scope: the Claude and Codex Push encoders, the Push builder in
  `service.rs`, one line of the Keep both apply, tests, and three documents.
  No frontend change. No Pull change other than that one line.
- Constraints: no customers, clean cutover, no compatibility path. Every
  schema stays as it is (section 9). An ordinary Push does no more work than
  today. No request to the storage is added.
- Builds on the session-sync work that is uncommitted in the owner's
  checkout: `edited_since_acceptance`, `PathOrigins::accepted_records_hold`,
  and `session_refresh_regression_tests.rs`. If any of them is missing, stop.
- Related: `PLAN_CLAUDE_DESKTOP_IMPORT_TOLERANCE.md` (the decision this plan
  carries into Push), `CURRENT_METADATA_LAYOUT.md` section 5.6,
  `formal/pathmap-origins-v1/README.md`.
- Line numbers are those of the working tree of 2026-09-29.

## 1. The defect

After the Claude desktop app imports a session that Mallard has already
pushed, every later Push of that session fails:

```
Claude record 2: retained transcript record changed; path origins no longer match
```

Refresh shows the session as Local ahead, so the user is sent in a circle.
The same guard stops the first Push of the copy that Keep both preserves,
for Claude and for Codex, whenever that copy was edited.

## 2. What is true today

| # | Fact | Where |
|---|---|---|
| 1 | A saved origin is found by the position of its record in the file, and holds a digest of that record. | `session_path_origins.rs:230` (`record`), `:122` (`verify_record`) |
| 2 | The Claude encoder asks for the origin at each record's position and fails on the first digest that differs. After the last record it fails when the file has fewer records than were saved. | `providers/claude/publication.rs:65`, `:101` |
| 3 | The Codex builder does the same per physical segment, and checks derived display records by digest before it omits them. | `codex_standalone_builder.rs:658-672`, `:724` |
| 4 | An import deletes rows that hold only thinking and strips thinking from the others. Every row after the first deleted one is at a new position. | `providers/claude/codec.rs:165` (`project_claude_record`) |
| 5 | Push has no other check that the accepted part of a session is unchanged. The review's relation is decided by counts; the publication is validated for its layout only. | `conversation_tail_relation.rs:255`, `mallard-domain/src/lib.rs:2146` |
| 6 | The wire fingerprint of a message hashes the whole projected record except `sessionId`, `cwd`, `uuid`, `parentUuid`, `timestamp`, `version` and `requestId`. Path references and redaction are inside it. | `providers/claude/codec.rs:1034` |
| 7 | Keep both moves the binding to the kept copy: it keeps its saved origins, loses its cloud mapping, gains `preserved`. Its checkpoint loses the accepted version. | `service.rs:12789`, `:12887` |
| 8 | The origin file is written while a Push is prepared. The binding takes the reference in the publication commit, never before. | `service.rs:10170`, `:10231` |
| 9 | Keep both re-creates the tracked session's checkpoint at generation 0, because the old checkpoint has been moved away before the generation is read. | `service.rs:12891`, `:12900` |
| 10 | A cached observation answers a query on checkpoint generation and binding revision only. | `observation_cache.rs:70` |

## 3. What was measured

Temporary tests in a scratch copy of the workspace, synthetic transcripts,
temporary homes. They are not in the tree.

| Case | Result |
|---|---|
| Push, import, continue; the file encoded with no saved origins at all | The wire form folds to the accepted wire value at the accepted end. The local form folds to the accepted local value. |
| Edit, Keep both, push the kept copy | Fails for Claude ("Claude record 1: retained transcript record changed") and for Codex ("Codex segment … record 2: …"). The kept copy has `preserved`, no cloud mapping, saved origins, and no accepted version. |
| Edit found by Refresh, Keep both, Refresh | Claude: `Diverged` from the cache, `Synced` once the cache is cleared. |
| Pull, import, Refresh | `Unknown`, Push and Pull `Blocked`: "invalid Claude installed discovery cutoff". |

The last row is outside this plan (section 10).

## 4. Decisions

1. **An import is not an edit, for Push as for comparison.** The owner
   decided it for comparison on 2026-08-20. This plan applies it to Push.
2. **No stored transcript is read.** The path codec already says that
   adoption "does not add a cloud transcript download" and that "no cloud
   request is added to normal Push or Pull". Both stay true.
3. **Saved edits are never replayed on a record whose digest differs.** A
   record for which no saved origin is found is scanned as a new record.
   "Stored origins always pass their hash check before inverse replay"
   stays true.
4. **A mapped session may do that only with two proofs.** The messages up to
   the accepted end give the accepted local value, and the new wire form
   gives the accepted wire value at the same end. Claude only.
5. **The first Push of a kept copy may do that with no proof.** It publishes
   under a new cloud identity. All four conditions must hold: the reviewed
   action is `CreateCloud`, the binding has `preserved`, the binding has no
   cloud mapping, the checkpoint has no accepted version. Both providers,
   and only the kept copy's own file.
6. **Accepted risk.** A record that arrived by Pull, was rewritten by the
   import, and is no longer part of the conversation gets new origins that
   nothing proves. When its text names this computer's project directory
   although it was plain text where it was written, the text comes back on
   the other computer naming that computer's directory. A path written in
   its resolved spelling can come back in the plain spelling. The owner
   accepts both. Of a row that is part of the conversation, the wire proof
   covers what the wire fingerprint hashes, which is the message. Its
   `cwd`, identifiers, time and thinking are outside the fingerprint, for
   this proof as for comparison. Section 14 ends this loss for a record
   that the import rewrote. It is left for a record that was rewritten in
   another way.
7. **A failed wire proof refuses the session as today.** The message and the
   saved state are those of the refusal the first encoding gave. See
   section 10 for why nothing more is done here.
8. **Child transcripts are not changed.** They keep today's check and sync
   as today (section 10).
9. **A mapped Codex session keeps the strict check.** No writer is known
   that changes Codex records and leaves the messages as they were.

## 5. The rule

```
encode, saved origins by position            (today's pass, unchanged)
  done                                       publish
  a saved record changed or is missing
    kept copy, all four conditions           encode again, by digest; publish
    Claude, accepted prefix proven (local)   encode again, by digest
        wire value proven at accepted end    publish
        not proven                           refuse, as today
    anything else                            refuse, as today
```

"Refuse, as today" includes today's next step: a session that was edited
since it was accepted gets `PUSH_EDITED` and loses the file state of its
cursor (`service.rs:8981`, `:10521`).

**By digest** means: walk the records of the file in order. For each one,
take the next unused saved origin that carries its digest and lies after the
last one taken. If there is one, use it exactly as a positional origin is
used today. If there is none, scan the record as a new record. The check of
the saved length is not made in this pass; the two proofs stand in for it.

## 6. Steps

### Step 0. Keep both must not restart the generation

This is a defect of the uncommitted edit detection, not of this plan. Step 5
cannot be tested honestly without it.

Read the generation of the tracked session's checkpoint before that
checkpoint is moved to the kept copy (`service.rs:12891-12905`). The
re-created checkpoint then continues the numbering, and no cached
observation of the old file can answer for it.

Test, Claude: an edit that Refresh finds itself, then Keep both, then
Refresh. The states are the same with the cache and with the cache cleared
(`assert_matches_fresh_observation`). The existing test does not reach this
case, because its refused Push raises the generation first.

### Step 1. A typed refusal

The encoders say in a type, not in a message, that a saved record changed
or is missing. Every other failure stays what it is: malformed JSON, a bad
token, failed exact restoration, a damaged or foreign origin file. The
printed text does not change.

Files: `session_path_origins.rs`, `session_path_map.rs`,
`providers/claude/publication.rs`, `codex_standalone_builder.rs`.
`providers/claude/subagents.rs` adapts to the type and keeps its behaviour.

### Step 2. Saved origins by digest

In `session_path_origins.rs`, beside the positional lookup. It is built only
for the second encoding, so an ordinary Push never builds it.

For Codex the `derived` flag of a saved origin is used only when the digest
matches. A record without a match is classified from its source by the
projection, like a record that was never covered.

### Step 3. `accepted_prefix_matches`

A pure function taken out of `edited_since_acceptance` (`service.rs:4169`):
the fold of the chain at the accepted end equals the accepted value. The
edit rule calls it; so does step 4. Permission is this proof, stated
positively. "Not edited" is never permission, and a cached observation is
never evidence.

### Step 4. The Claude builder

In `build_conversation_publication` (`service.rs:9308-9489`), around the one
call of `encode_main_transcript`:

1. On the typed refusal, prove the accepted prefix in local form, from the
   chain of the captured transcript and the checkpoint's
   `accepted_local_tail`.
2. Encode once more, by digest.
3. Fold the wire chain of that encoding at the accepted end and compare it
   with the checkpoint's accepted wire value. When the export settings
   differ from those of the stored version, make this proof on one more
   encoding under the stored version's redaction stamp, as first-contact
   adoption does.
4. Publish the second encoding. Its origins are the ones saved.

The freshness checks at the end of the builder are unchanged, so a
transcript that changed after capture still fails.

### Step 5. The kept copy

The same second encoding, without proofs, when the four conditions of
decision 5 hold. Claude: the main transcript. Codex: the leaf segment of the
root; segments of ancestors and every child keep the strict check. The
prefix witness is unchanged: a kept copy that was edited before its fixed
start stays refused.

After its first Push the copy is an ordinary mapped session.

### Step 6. Tests

Beside the code they protect; the service cases go into
`session_refresh_regression_tests.rs`.

| Case | Required result |
|---|---|
| Push, import, Refresh | Synced (exists) |
| Push, import, continue, Push | Success, then Synced. `a_desktop_import_changes_no_message_and_is_no_edit` asserts this instead of the failure. |
| The Push after that one | Rides the cursor; one encoding pass; no lookup by digest |
| Import removes more records than the continuation adds | Success |
| Row with thinking and a project path, created here | Wire text of the row unchanged but for the thinking |
| Edit of an earlier message with growth, both providers | Refused once, then Diverged (exists, unchanged) |
| Claude rewind | Success (exists, unchanged) |
| Edit, Keep both, push the kept copy, both providers | Success under a new cloud identity; both copies stay |
| Continue the kept copy, Push | Success on the ordinary path |
| Later edit of the pushed copy, with growth | Refused once, then Diverged |
| Local-only session with saved origins and no `preserved` | Refused, as today |
| Corrupt, missing or foreign origin file | Refused, acceptance unchanged (exists) |
| Publication fails after the second encoding | The binding keeps its old origins reference; what was accepted is unchanged. The cursor follows the file, as in every Push. |
| Ordinary append Push | Same counts of encoding passes and record hashes as today |
| Pulled row, rewritten, outside the conversation, names the local project directory | Published as a reference. This test records decision 6. |
| The same row inside the conversation | Refused; nothing published |
| Records by digest: moved, rewritten, repeated, out of order | Unit test of step 2 |
| Codex record whose saved `derived` flag no longer matches its digest | Classified from its source |

The two cases of a pulled row need two bindings with different project
directories, as `claude_round_trip_scrubs_secrets_end_to_end` has, so that
the text is plain on the first and names the project on the second. They
also need a session that Pull installed. Until section 13 an import of one
was refused at Refresh, and the tests padded the rewritten row with JSON
whitespace so that every byte offset after it stayed where it was. They now
import the whole transcript.

### Step 7. Documents

Section 8.

### Step 8. Verification

```bash
cargo test --manifest-path src-tauri/Cargo.toml --lib -- a_desktop_import a_claude_desktop_import
cargo test --workspace --all-targets
npm run test:frontend-integration
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
git diff --check
```

One measurement, release build, a synthetic session of 10 MiB: the time of
an ordinary append Push before and after the change, and the time of a Push
that needs the second encoding. No transfer is measured, because none is
added.

Review the final diff against the dirty tree the work started from. Do not
commit or push.

## 7. What an ordinary Push costs

Nothing more. The first encoding is today's. The lookup by digest, the two
proofs and the second encoding exist only after the typed refusal.

A Push that needs them costs one more encoding of that one session. With
changed export settings it costs a third.

## 8. What this reverses

| Passage | Today | After |
|---|---|---|
| `formal/pathmap-origins-v1/README.md:3-5` | Credential redaction is the one exception to returned text. | A second exception: decision 6. |
| `formal/pathmap-origins-v1/README.md:66-70` | "Missing, corrupt, stale, swapped, truncated, or misaligned referenced data fails the affected thread." "It scans only new records." | Still so for missing and corrupt data. A record with no saved origin of its digest is scanned as new under decision 4 or 5. |
| `formal/pathmap-origins-v1/README.md:150-153` | Blind rescanning is a negative control. | Unchanged, and cited: decision 6 is that counterexample, accepted for records outside the conversation. The models are not changed. |
| `CURRENT_METADATA_LAYOUT.md:809-824` | "Push reuses them for retained history … It scans new records." "Missing or stale origins fail the affected thread." | The same two sentences with the two cases added. |
| `session_path_map.rs:1-2` | "Lossless project-path references." | Names the exception. |
| `session_refresh_regression_tests.rs:719-723` | The comment says Push refuses the rewritten records. | Removed with the assertion. |

The observation cache is schema 1, in the code and in
`CURRENT_METADATA_LAYOUT.md:144`. The uncommitted session-sync work had
raised the number to 3. The owner reset it on 2026-09-29: "Reset the schema
to 1, there is no customer." A file whose records have another shape does
not parse and is discarded.

## 9. Metadata

No format changes and no schema number changes, on the computer or in the
storage.

| Metadata | Change |
|---|---|
| `path_origins/<sha256>.json` | Format unchanged, schema 1. A Push that needed the second encoding writes one new file, as every Push does. In it, a rewritten record has origins from a new scan; a moved record has the origins it had. |
| `conversation_bindings.json` | No new field. The pushed row takes the new origins reference in the publication commit, as today. A kept copy takes its minted cloud mapping there, as any first Push does. |
| Checkpoints in the sync config | No new field. Written by the Push as today. Step 0 changes one value: the checkpoint that Keep both re-creates continues the generation instead of starting at 0. |
| `observation_cache.json` | Unchanged by this plan. Schema 1 (section 8). |
| Bundle manifest, publication record | Unchanged, schema 1. The thread row is rewritten as by any Push. A kept copy publishes its `preserved` provenance, which the code already does (`service.rs:8589`). |
| Stored transcript | A new object. The previous one stays, as always. |
| A refused Push | Writes what it writes today: nothing, or the cleared cursor file state of an edited session. |

Nothing has to be reset. Origins saved before this change are read as they
are. Section 13 adds one field to `conversation_bindings.json`. Section 14
adds one optional member to a record of `path_origins/<sha256>.json`.

## 10. Not in this change

- **Import after Pull.** The installed cutoff is a byte offset
  (`installed_end`, set at `service.rs:12828`), and the import moves every
  byte. Refresh refuses the session before Push is reached
  (`service.rs:4960`), and Refresh writes no binding, so no repair inside
  Push can help. The cutoff needs a form that survives the rewrite. That is
  a change of `conversation_bindings.json`. Done in section 13.
- **Refresh and a failed wire proof.** Refresh judges the local form and
  never encodes, so it cannot see a failed wire proof and would show Local
  ahead again. For a session that was never installed by Pull the new scan
  gives what the saved origins gave, so the proof cannot fail through an
  import. Section 13 makes it reachable and says what the user meets. It
  is not solved.
- **Child transcripts.** They sync as they do today; this plan changes
  nothing for them. A session with children is pushed after an import like
  any other, as long as the import left the child files alone: the links to
  the children are in user rows, which the import does not touch, and each
  child passes today's check. Whether the import rewrites child files is
  not known (open item 1). If it does, a child fails today's check and the
  whole session is refused, as it is today. The rule of step 4 is then
  applied to each child, with the child's own accepted state
  (`accepted_subagents`, a message count and a hash in both forms) as the
  two proofs.
- **The stored transcript as a reference.** Decisions 2 and 6.

## 11. Open before the work is called complete

1. One real import in an isolated `CLAUDE_CONFIG_DIR`: are child files
   rewritten, and do numbers change their form? The gate S6 of
   `PLAN_CLAUDE_DESKTOP_IMPORT_TOLERANCE.md` never ran.
2. A count, over real transcripts, of rows that hold thinking together with
   other blocks. If there are none, an import rewrites no record and
   decision 6 has no case.
3. The owner's live pass in the app.

## 12. What was built

Implemented on 2026-09-29. Child transcripts were left out on the owner's
word. Nothing is committed.

### Code

| Step | What | Where |
|---|---|---|
| 0 | Keep both reads the generation before it moves the checkpoint. | `service.rs`, Pull apply |
| 1 | `EncodeRefusal { message, saved_record_changed }`. The printed text is unchanged. | `session_path_origins.rs`; returned by `verify_record`, `replay`, `verify_source_length`, `encode_record_with_origins`, `encode_main_transcript`, `encode_codex_tree_member` |
| 2 | `OriginLookup::{ByPosition, ByDigest}`, `PathOrigins::saved`, `SavedOrigins::origin_of`. | `session_path_origins.rs` |
| 3 | `accepted_prefix_matches`. | `service.rs` |
| 4 | `encode_rewritten_claude_transcript`. | `service.rs` |
| 5 | `first_push_of_a_kept_copy`; the Codex builder takes the lookup of its leaf segment. | `service.rs`, `codex_standalone_builder.rs` |
| 7 | The documents of section 8. | |

No schema number and no file format changed.

### Corrections to the plan

1. **Decision 6.** The wire proof covers what the wire fingerprint hashes.
   The plan said that a row of the conversation "is never published
   changed". Its `cwd`, identifiers, time and thinking are outside the
   fingerprint.
2. **Step 6, a failed publication.** The plan said the checkpoint is
   unchanged. What was accepted is unchanged. The cursor follows the file,
   as it does in every Push of a mapped session.
3. **The proof in the local form.** In every flow a user can reach, a
   session whose messages changed is stopped before the encoder: by the
   check of the reviewed state when Refresh read through the cursor, and by
   the edit rule when Refresh read in full. The proof is kept. It costs one
   fold and no encoding, and it is the only check that tells apart two keys
   that redaction stores as the same text.

### Found while testing, and not changed

- **A rewind after an import reads as an edit.** The session shows Diverged
  and Pull offers Keep both. The kept copy can then be pushed. Without an
  import a rewind stays local progress. The cause is the third condition of
  `edited_since_acceptance`: the record digests, which an import changes.
- **The tree is not clean for the two style checks.** `cargo fmt --all
  --check` reports 37 files and `cargo clippy --workspace --all-targets --
  -D warnings` stops at `mallard-domain/src/lib.rs:3564`. Both are so
  without this change. This change adds nothing to either.
- **One test of `mallard-cli` fails now and then.**
  `raw_injected_callback_canary_cannot_complete_or_reach_output` failed
  once with a reset loopback connection and passed in seven other runs.

### Tests

| Case of section 6 | Test |
|---|---|
| Push, import, continue, Push; the Push after it; a row with thinking and a project path | `a_desktop_import_changes_no_message_and_is_no_edit` |
| Import removes more records than were added | `a_desktop_import_that_shortens_the_transcript_is_pushed` |
| Redaction setting changed since the stored version | `an_imported_session_is_proven_under_the_redaction_it_was_stored_with` |
| The rule, with its encodings counted | `a_transcript_is_encoded_again_only_with_what_permits_it` |
| Publication fails after the second encoding | `a_failed_publication_of_an_imported_session_saves_nothing` |
| Kept copy: first Push, continued, edited later; both providers | `the_copy_that_keep_both_preserved_is_pushed` |
| Local-only session without `preserved`; both providers | `a_local_session_that_no_keep_both_preserved_stays_refused` |
| Pulled row, rewritten, outside the conversation | `a_row_rewritten_in_another_way_outside_the_conversation_is_scanned_as_new` (renamed in section 14) |
| The same row in the conversation | `a_row_rewritten_in_another_way_in_the_conversation_is_refused` (renamed in section 14) |
| Step 0 | `keep_both_leaves_no_observation_of_the_file_it_moved` |
| An edit that redaction hides | `an_edit_that_redaction_hides_is_an_edit` |
| Records by digest | `a_record_finds_by_digest_the_origin_saved_of_its_content` |
| The typed refusal | `a_changed_or_missing_saved_record_is_told_apart_from_other_refusals`, `a_missing_record_is_told_apart_from_a_transcript_that_cannot_be_read` |
| The Claude encoder, both lookups | `a_rewritten_transcript_is_refused_by_position_and_encoded_by_digest` |
| Ordinary append: passes and hashes | `an_appended_transcript_is_encoded_in_one_pass_with_one_hash_a_record` |
| Codex display record that was changed | `a_changed_display_record_is_classified_from_its_content_by_digest` |

The edit with growth, the rewind, and the damaged origin file keep the tests
they had, unchanged.

Each condition of the rule was removed once, and the test named for it
failed each time: the generation of step 0, the mark of Keep both, the
proof in the local form, the proof in the wire form, and the encoding under
the stored redaction.

### Verification

| Check | Result |
|---|---|
| `cargo test --workspace --all-targets` | Passed: 1,481 tests, 0 failed, 23 ignored. Backend library 464, infrastructure 780, domain 101. |
| `npm run test:frontend-integration` | Passed: 470 tests, 0 failed. |
| `git diff --check` | Clean. |
| `cargo fmt --all --check` | 37 files reported, all of them before this change. |
| `cargo clippy --workspace --all-targets` | No warning on a line of this change. |

### Measurement

Release build, temporary home, local-folder storage, one synthetic Claude
session of 11.5 MB and 8,321 records. Each figure is the review and the
Push together. Three runs of each build.

| Push | Before | After |
|---|---|---|
| First Push | 691 to 771 ms | 673 to 704 ms |
| Appended session | 594 to 640 ms | 575 to 599 ms |
| Imported and appended session, 9.6 MB after the import | refused | 668 to 682 ms |
| Appended session after that | | 504 to 534 ms |

An ordinary Push costs what it cost. The Push that follows an import costs
about 150 ms more than an ordinary Push of the same session, once.

```bash
cargo test --release --manifest-path src-tauri/Cargo.toml --lib -- \
    --ignored push_cost_sample --nocapture
```

### Still open

Section 11 is unchanged: one real import in an isolated
`CLAUDE_CONFIG_DIR`, the count over real transcripts, and the owner's live
pass. When this section was written, an import of a session that Pull
installed was refused at Refresh by its installed cutoff, before Push was
reached. Section 13 changes that.

## 13. Import of a session that Pull installed

Implemented on 2026-09-29, on the owner's word ("Implement the cutoff fix
first"). Not committed.

### The defect

After the desktop app imports a session that Pull installed, Refresh shows
the session as Unknown, and Push and Pull are blocked:

```
This conversation's local session could not be read for review: invalid Claude installed discovery cutoff
```

A session that was made on the computer is not affected. Its binding has no
cutoff.

### The cause

Pull saved the byte length of the file it wrote, as `installed_end`. Two
checks read it as a place in the file, and an import moves every byte. In
the reproduction the cutoff was 3,443 and the imported file had 1,225 bytes.

| Check | Where it ran | Result after an import |
|---|---|---|
| The cutoff is in the file and follows the end of a record (`validate_installed_cutoff`) | Refresh, full read and cold membership scan; the Push builder | Refresh fails before the conversation is compared |
| The cutoff is not beyond the complete history (`candidates`) | Child discovery, in Refresh and Push | No child membership; the session is Unknown |

Either check alone gives Unknown.

### What the cutoff was for

It told the child references that came with a Pull from those made on the
computer since. Only one kind of reference needed it.

| Child reference in a pulled file | How it is known | Needed the cutoff |
|---|---|---|
| Child that Pull installed | By its ID, from `installed_subagents` | No |
| Copied from an ancestor session | By its session ID, which Pull keeps | No |
| Came without its transcript | By its place before the cutoff | Yes |

A reference comes without its transcript when the other computer pushed
with children sync off, or left all children out.

### The rule

A child that came with a Pull is known by its ID, never by its place in the
file.

1. Pull saves the provider IDs of the references that came without a
   transcript, in `installed_absent_subagents`: the references of the
   installed main transcript, less the children it installed.
2. Child discovery skips a reference whose ID is an installed child or is
   in that list. Every other reference is a new child, as before.
3. No byte check is left for Claude. Codex is unchanged: a rollout is only
   appended to, and its discovery reads the rollout after `installed_end`.
4. `installed_end` stays. It marks a binding that Pull installed, and Pull
   apply checks it against the bytes it writes, for both providers.
5. Keep both moves the binding to the kept copy with its list, as it moves
   the file.

### Code

| What | Where |
|---|---|
| `installed_absent_subagents: BTreeSet<String>` on `LocalConversationBinding`, with its checks: only on an installed binding, bounded, no installed child among them, Claude only | `mallard-domain/src/lib.rs` |
| `candidates` skips the absent IDs and has no byte check. `validate_installed_cutoff` is removed | `providers/claude/subagents.rs` |
| `ChildLinks` maps an agent ID to its key. `ChildLink` and its offset are removed, and `record_link` and the record callback of the tail refresh lose the offset | `providers/claude/subagents.rs`, `claude_history_tail.rs`, `providers/claude/publication.rs`, `pull_transcript.rs` |
| `ChildReferences::absent`; `ChildReferences::prepare` borrows | `owned_subagents.rs` |
| The three calls of `validate_installed_cutoff` are removed. `apply_prepared_pull_conversation` computes the list after the loop over the children and writes it in the three binding writes. The binding that Push creates has an empty list | `service.rs` |

### Metadata

| Metadata | Change |
|---|---|
| `conversation_bindings.json` | One field, `installed_absent_subagents`, written only when it has entries. 22 bytes for each reference in the measurement. |
| `observation_cache.json` | Schema 1, as before. A saved child reference had a key and a place, and has a key now. A saved record of the earlier shape does not parse, so the file starts empty and each session is read once. |
| Everything else | Unchanged. Nothing changes in the storage. |

A binding that was written before this change has no list. If its session
holds references that came without transcripts, they are taken for new
children. With children sync off nothing follows from that. With children
sync on, the session shows a missing child, or its Push is refused, until
a Pull installs it again.

### What this reverses

| Passage | Before | After |
|---|---|---|
| `PLAN_CLAUDE_OWNED_SUBAGENT_SYNC.md`, "For a task installed by Pull" | "Discover new children only after the main's `installed_end` byte offset." "A stale cutoff or invalid installation map is an error." | Children that came with a Pull are known by ID. The passage has a dated note. |
| Section 10 of this plan, "Import after Pull" | Not in the change. | Done. |
| `CURRENT_METADATA_LAYOUT.md`, section 3.3 | The installation fields were not described. | Described, with the rule of both providers. |

### Tests

| Case | Test |
|---|---|
| Pull, continue, Push, import, Refresh, continue, Push; the first computer pulls the result | `a_desktop_import_of_a_pulled_session_is_no_edit_and_is_pushed` |
| The same with two installed children and one made since | `children_that_came_with_a_pull_are_known_after_a_desktop_import` |
| The same with two references that came without transcripts | `a_reference_that_came_without_its_transcript_is_no_new_child` |
| Discovery by ID, with a cutoff beyond the end of the file | `a_child_that_came_with_a_pull_is_known_by_its_id` |
| The checks of the new field, and its form in the file | `conversation_bindings_carry_validated_layouts` |
| A Codex binding with the list is refused | `only_a_claude_binding_names_absent_children` |
| A saved observation of the earlier shape | `a_saved_observation_of_the_earlier_shape_is_read_again` |
| A saved record of another shape under the current schema number | `an_unreadable_or_older_schema_file_starts_empty` |
| The two cases of a pulled row, now with a whole import and no padding | `a_row_rewritten_in_another_way_outside_the_conversation_is_scanned_as_new`, `a_row_rewritten_in_another_way_in_the_conversation_is_refused` (renamed in section 14) |
| Cost | `cost_sample_of_a_pulled_session`, ignored |

In every one of the first three tests the imported file is shorter than the
file Pull wrote.

Each part of the fix was removed once on the prototype, which has the same
product code, and a test failed each time: the skip of the absent IDs, the
list that Pull saves, the byte check in Refresh, and the byte check in
`candidates`.

### Verification

| Check | Result |
|---|---|
| `cargo test --workspace --all-targets` | Passed: 1,489 tests, 0 failed, 25 ignored. Backend library 468, infrastructure 782, domain 101. |
| `npm run test:frontend-integration` | Passed: 470 tests, 0 failed. |
| `node scripts/run-live-claude-aimock-e2e.mjs --scenario desktop-import`, with Claude Code 2.1.266 | Passed: 19 rounds, 4 imports. It failed at round 10 before. |
| `git diff --check` | Clean. |
| `rustfmt --check` on each changed file | Clean where the file was clean before. `mallard-domain/src/lib.rs`, `mallard-domain/src/tests.rs` and `persistence.rs` were not clean before, and the count of their lines that rustfmt would change is the same. |
| `cargo clippy --workspace --all-targets` | No warning on a line of this change. |

### Cost

Release build, temporary home, local-folder storage, the prototype in a
scratch copy. One pulled Claude session of 11.6 MB and 9,059 records, with
226 child references that came without transcripts. The median of three
runs, with the range.

| Step | Before | After |
|---|---|---|
| Pull | 622 ms (596 to 634) | 602 ms (580 to 686) |
| First Refresh after the Pull | 105 ms (104 to 109) | 104 ms (102 to 119) |
| Refresh, nothing changed | 3.4 ms (2.8 to 5.3) | 3.6 ms (2.6 to 5.9) |
| Refresh, nothing changed, no saved observation | 99 ms (93 to 130) | 97 ms (94 to 112) |
| Refresh of an appended session | 27 ms (20 to 120) | 26 ms (23 to 122) |
| Push of an appended session | 647 ms (576 to 786) | 599 ms (567 to 676) |
| One read of the bindings document | 0.67 ms | 0.68 ms |
| First Refresh after the import | refused | 104 ms |
| Push of an imported and appended session | refused | 697 ms (690 to 711) |
| Push of an appended session after that | | 545 ms (539 to 562) |

The differences are inside the noise. The fix adds one lookup for each
child reference and reads no file.

```bash
cargo test --release --manifest-path src-tauri/Cargo.toml --lib -- \
    --ignored cost_sample_of_a_pulled_session --nocapture
MALLARD_COST_SAMPLE_IMPORT=1 cargo test --release --manifest-path src-tauri/Cargo.toml --lib -- \
    --ignored cost_sample_of_a_pulled_session --nocapture
```

### Still open

- **The refused case on the conversation path can now be reached.** It
  needs a pulled session, an import, a record of the conversation that
  holds thinking and visible text together, and in that record a project
  path that a new scan stores in another form. The user then meets this:
  the session shows Local ahead and Push offers it; Push fails with
  "retained transcript record changed; path origins no longer match"; the
  session shows Local ahead again, and every Push fails the same way. The
  session file is not changed. A way out opens only when another computer
  pushes a newer version: the session is Diverged, Pull offers Keep both,
  and the kept copy is pushed as a separate session. Claude Code 2.1.266
  wrote every thinking block in a record of its own in the end-to-end
  run, so an import deleted records and rewrote none. Fixed in section 14
  for a session that is pulled after that change.
- **A Pull on the computer that made a session with child agents fails**
  when the children are stored: "child destination is occupied". Pull
  apply knows a child target only through `installed_subagents`, and the
  binding of a session made on the computer has none. No import is
  involved. Fixed in section 14.
- **Child transcripts and an import.** Unchanged from section 10.
- **Section 11.** Unchanged: one real import in an isolated
  `CLAUDE_CONFIG_DIR`, the count over real transcripts, and the owner's
  live pass.

## 14. Pull on the computer that made the children, and the rows an import rewrote

Implemented on 2026-09-29, on the owner's word ("Implement all three"). Not
committed. Three changes. The first two are in Pull and need no import. The
third ends the refused case of section 13.

### The defects

| # | What the user meets | What it needs |
|---|---|---|
| 1 | The computer that made a session with child agents cannot pull a later version of it: "child destination is occupied". | Stored children. No import. |
| 2 | The same computer pulls a later version of the main transcript alone. Afterwards the session page shows no agent cards, and the children are never stored, also after children sync is turned on. Their transcripts are on the computer. | Children sync off, which is the default. No import. |
| 3 | After an import, every Push of a pulled session fails: "retained transcript record changed; path origins no longer match". | A pulled record that holds thinking beside other content, with a project path that a new scan stores in another form. |

### The causes

1. Pull apply knew a child target only through `installed_subagents`, and
   only a Pull fills that map. The binding of a session made on the
   computer has none, so the file that the provider wrote there counted as
   a foreign file in the way.
2. Pull apply took every reference of the installed main transcript whose
   child it did not install for one that came without its transcript. A
   child made on the computer is such a reference whenever the stored
   version holds no children.
3. The import takes the thinking out of the record. The saved origins were
   of the record with its thinking, so none carried the digest of the new
   form, and the record was scanned as new. A new scan stores a reference
   where the accepted text was plain.

### The rules

1. **An update in place replaces a child that is known here, under the
   state it was reviewed in.** A child is known by the Pull that installed
   it, or by the reference of the local main transcript when the provider
   wrote its file on this computer. The review pins the file of each known
   child that the stored version holds. Apply takes the expectation of a
   child from its pin. A target without a pin must be free. The engine
   keeps a backup of each file it replaces, as it does for the main
   transcript.
2. **A reference stays a child when no Pull brought it and its transcript
   is here.** The binding says what a Pull brought, as it was before this
   Pull: the ID is an installed child, or it is in
   `installed_absent_subagents`. A child made here is left out of the list
   that the Pull writes, and `pulled_children_excluded` is false when
   there is one. A file alone decides nothing: a copy that an earlier Pull
   installed stays on disk after a Pull of a version without children, and
   it can be older than the main transcript.
3. **Pull saves the origins of a record in both forms.** For a Claude
   record that holds thinking beside other content it saves the origins of
   the record, and under `rewritten` those of the record without its
   thinking: the digest of that form, and the saved edits outside the
   thinking, at the places the content then has. The lookup by digest knows
   both digests, and the two forms are one place in the order of the file.
   The lookup by position gives the origins of the record itself, as
   before. Push saves no second form: a new scan of a record that was
   written on the computer gives what its saved edits give.

Codex is not affected by the first two. A Codex update is written under a
new identity, and its discovery reads the rollout after `installed_end`.

What stays refused:

| Case | Result |
|---|---|
| A file lies where a stored child belongs, and the local main transcript does not refer to it | "child destination is occupied". The file is left as it is. |
| A child was continued after the review | "This conversation changed after the comparison. Compare again." |
| The file of a child changed during the Pull, in bytes that change no message | Refused before the download by the check of the pins, and after it by the engine. The file is left as it is. |
| A record of the conversation that was rewritten in a way Mallard does not know | Refused, as in section 13 |
| Such a record outside the conversation | Scanned as new: decision 6 |

### Code

| What | Where |
|---|---|
| The review pins a child by the installed map, or by the observed local tree | `service.rs`, `provider_thread_review`, `MaterializeCloud` |
| Apply takes the expectation of a child from its pin. A target without one must be absent | `service.rs`, `apply_prepared_pull_conversation` |
| The children made here are taken out of the absent list, and `pulled_children_excluded` follows them. The three binding writes take both | The same function |
| `RecordPathOrigins.rewritten`. `SavedOrigins::origin_of` and the index by digest know both digests | `session_path_origins.rs` |
| `origins_without_thinking` | `providers/claude/codec.rs` |
| Pull saves the second form | `pull_transcript.rs` |

Product code: `service.rs` 74 lines added and 21 removed; the three files
of the infrastructure crate 99 added and 13 removed.

### Metadata

| Metadata | Change |
|---|---|
| `path_origins/<sha256>.json` | One optional member of a record, `rewritten`, written only for a Claude record that holds thinking beside other content. Schema 1. A file that was written before has none and is read as before. |
| `conversation_bindings.json` | No new field. After an update in place, `installed_absent_subagents` leaves out the children made on the computer, and `pulled_children_excluded` is false when there is one. |
| Everything else | Unchanged. Nothing changes in the storage. |

A session that was pulled before this change has origins of one form. An
import of it behaves as in section 13 until a Pull installs it again.

### What this reverses

| Passage | Before | After |
|---|---|---|
| Decision 6 | A rewritten record outside the conversation is scanned as new. | Only when no saved origin carries its digest. A record that the import rewrote finds the origins saved of that form. |
| Section 13, "Still open", the first two items | Open. | Done. |
| `PLAN_CLAUDE_OWNED_SUBAGENT_SYNC.md`, "Pull preserves Claude agent IDs" | "retain that binding's parent ID and known child mappings. Reject occupied unrelated targets." | A known child is also one that the local transcript refers to and the provider wrote here. The passage has a dated note. |
| `formal/pathmap-origins-v1/README.md`, the entries of a file and "Rewritten transcripts" | An entry has a hash, a flag and edits. Every rewritten record without a saved origin is a loss. | An entry can have a second one. The loss is left for a record that finds no entry of its hash. |
| `CURRENT_METADATA_LAYOUT.md`, sections 3.3 and 5.6 | The absent list was the references less the installed children. | Less the children made on the computer too. The second form is described. |
| `session_path_map.rs`, the first lines | "a record that was rewritten after its origin was saved is scanned as a new record" | Only a form of which no origin was saved. |

### Tests

The service cases are in `session_refresh_regression_tests.rs`, on the
fixture `TwoComputers`, which now takes a provider.

| Case | Test |
|---|---|
| Push with children, Pull on the second computer, a child continued there, Push; the computer that made the session pulls. Both providers | `the_computer_that_made_a_session_pulls_its_children` |
| A file in the place of a stored child, which the local transcript does not refer to | `a_file_that_the_session_does_not_refer_to_is_never_replaced` |
| A child continued after the review | `a_child_that_was_continued_after_the_review_is_not_replaced` |
| The file of a child changed before the download, and after it | `a_child_whose_file_changed_during_the_pull_is_not_replaced` |
| Children sync off: Push, Pull, Push, and the computer that made the children pulls. The session page, the membership, and the Push after children sync is turned on | `children_made_here_stay_children_after_a_pull_of_the_main_alone` |
| The same with one more child that was made on the second computer | `a_child_made_on_another_computer_came_without_its_transcript` |
| A copy that an earlier Pull installed stays hidden and is not stored | `pulled_children_excluded_hides_agent_cards_until_children_are_pulled`, `child_settings_stamp_scope_and_pull_lifecycle` (both existed, unchanged) |
| Pull, import, continue, Push, in the conversation and outside it | `a_row_that_the_import_rewrote_is_stored_as_it_was_accepted` |
| A row rewritten in another way, outside the conversation and in it | `a_row_rewritten_in_another_way_outside_the_conversation_is_scanned_as_new`, `a_row_rewritten_in_another_way_in_the_conversation_is_refused`. They are the two tests of a pulled row of section 12, renamed. The rewrite that Mallard does not know is a changed `timestamp`. |
| The second form: places of the edits, both directions, the records that have none | `a_record_without_its_thinking_is_encoded_from_the_origins_saved_for_it` |
| The lookup: both forms, one place, by position, a file without the member | `a_rewritten_record_finds_by_digest_the_origins_saved_of_that_form` |
| The Claude encoder gives the rewritten record the origins of its form | `a_rewritten_transcript_is_refused_by_position_and_encoded_by_digest` (existed; its fixture now saves what Pull saves) |
| Cost | `pull_cost_sample_of_a_session_with_children`, `cost_sample_of_rows_with_thinking_beside_other_content`, both ignored |

Each part of each rule was removed once, in a copy of the tree as it is, and
a test failed each time:

| Rule | Part that was removed | Tests that failed |
|---|---|---|
| 1 | The review pins a child that the local tree knows | 2 |
| 1 | Apply takes the expectation from the pin | 1 |
| 1 | A target without a pin must be free | 1 |
| 2 | A child made here is left out of the absent list | 2 |
| 2 | "No Pull brought it": a file alone made a child | 2, both existed before |
| 2 | "Its transcript is here": a reference alone made a child | 1 |
| 2 | `pulled_children_excluded` follows the children made here | 2 |
| 3 | The index by digest knows the second digest | 3 |
| 3 | Pull saves the second form | 1 |
| 3 | An edit moves to the place its block has then | 1 |
| 3 | An edit inside the thinking is left out | 1 |

### Verification

On the working tree, with a target directory of its own.

| Check | Result |
|---|---|
| `cargo test --workspace --all-targets` | Passed: 1,503 tests, 0 failed, 28 ignored. Backend library 476, infrastructure 784, domain 101. |
| `node scripts/run-live-claude-aimock-e2e.mjs --scenario desktop-import`, with Claude Code 2.1.266 | Passed: 19 rounds, 4 imports. |
| `git diff --check` | Clean. |
| `rustfmt --check` on each changed file | Clean. `session_path_map.rs` was not clean before, and the count of its lines that rustfmt would change is the same. |
| `cargo clippy --workspace --all-targets` | No warning on a line of this change. |
| `npm run test:frontend-integration` | Not run. No frontend file and no DTO changed. |

### Cost

Release build, temporary homes, local-folder storage, the prototype in a
scratch copy, which has the same product code. The computer was under load
from other builds, so the counts of bytes are exact and the times are
rough. The counts were taken again on the tree and are the same.

**Pull of a session with children.** One Claude session with 8 children of
1 MiB. The counts are of the files of the computer that pulls.

| Pull | Bytes read | Bytes hashed |
|---|---|---|
| The second computer creates the session | 8,397,020 | 8,397,020 |
| The computer that made the session, update in place. Refused before | 50,372,122 | 41,981,942 |
| The second computer, update in place. The path that existed | 50,372,907 | 41,982,989 |
| Any of them, no children | under 5,000 | under 7,000 |

Apply of an update in place took 540 to 1,330 ms with the children and 110
to 290 ms without them, on either computer. The review reads no child
file. The Pull that was refused now does what an update of a pulled session
did before: no work of a new kind. An update in place reads each child file
that it replaces six times and hashes it five times, and it replaces every
stored child, changed or not. Both are so without this change.

**Children made here.** One check that a file exists, for each reference
without a stored transcript that no Pull brought, at an update in place.

**The second form of the origins.** One Claude session of 11,528,442 bytes
and 8,275 records. One record in ten holds thinking beside text, and one in
ten holds thinking alone. Three runs of each build.

| Step | Before | After |
|---|---|---|
| Saved origins | 2,055,033 bytes | 2,333,732 bytes, 13.6% more |
| Pull | 1,166 to 1,995 ms | 1,272 to 1,614 ms |
| Push of an imported and appended session | 1,230 to 1,850 ms | 1,326 to 1,377 ms |

The second form costs 337 bytes for each record that has one, and one more
hash of that record at Pull. No difference in time could be measured. An
ordinary Push does no new work: the lookup by digest is built only for the
second encoding.

```bash
cargo test --release --manifest-path src-tauri/Cargo.toml --lib -- \
    --ignored pull_cost_sample --nocapture
cargo test --release --manifest-path src-tauri/Cargo.toml --lib -- \
    --ignored cost_sample_of_rows_with_thinking --nocapture
```

### Found while testing, and not changed

- **Children of one session that were made on two computers.** The
  comparison takes a session and its children as one unit: progress on
  this computer in one transcript and progress in the storage in another
  is Diverged. When each of two computers has children of its own that are
  not stored, and one computer stores its children, the other shows
  Diverged and Pull offers Keep both, although the main transcripts agree.
  Nothing is lost: the kept copy holds its children and is pushed as a
  session of its own. Before rule 2 this state could not build up, because
  an update in place dropped the children of the computer it ran on. A
  Pull that installs the stored children and keeps the local ones as local
  progress would need a change of the comparison. It is not in this change.
- **The session page shows all agent cards or none.**
  `pulled_children_excluded` is one flag for the session. On a computer
  that pulled a session without its children, a child that is made there
  afterwards has no card until an update in place finds it. When the flag
  is false, the cards of references without a transcript are shown too.
  `installed_absent_subagents` names the references without a transcript;
  the page does not read it.
- **A session that was pulled before this change** has origins of one
  form. Its refused case stays until a Pull installs it again.
- **One storage test fails now and then under load.**
  `streamed_staging_uses_an_inactivity_timeout_and_bulk_preserves_order`
  failed once in 14 runs of the infrastructure suite, with "timed out
  after 40 ms", while two suites ran at the same time. It passed in both
  runs on the tree. This change does not touch the storage transport.

### Still open

- **Child transcripts and an import.** Unchanged from section 10.
- **Section 11.** Unchanged: one real import in an isolated
  `CLAUDE_CONFIG_DIR`, the count over real transcripts, and the owner's
  live pass.

## 15. An agent card for each child, and the children of two computers

Implemented on 2026-09-29, on the owner's word ("implement your fix for
both A and B", "remove the deadcode, keep schema 1"). Not committed. It
changes the first two items of "Found while testing, and not changed" of
section 14. The owner set the cost aside for now ("I want correctness
first"), so no time was measured.

### The defects

| # | What the user meets | What it needs |
|---|---|---|
| 1 | A computer pulled a session without its children. A child agent that is made there afterwards has no card on the session page. Its transcript is on the computer, and Push stores it. | The stored version was pushed with children sync off, which is the default. |
| 2 | The session page shows cards that lead to no transcript: the references that came without one. | The Pull found a child that was made on the computer, or the stored version says that children are stored and holds none of them. |
| 3 | Each of two computers made a child of one session, and neither child is stored. One computer stores its children. The other shows Diverged, and Pull offers Keep both, although the main transcripts agree. The session becomes two. | Claude, and children sync turned on after the first Push. |

### The causes

1. The page asked one flag of the binding, `pulled_children_excluded`, for
   all cards of a session. The flag was true when the stored version was
   pushed without children and the Pull found no child made here. The
   binding has named each child since section 14, and the page did not
   read that.
2. The comparison folds the relations of the main transcript and of each
   child into one. It took any change here beside any change in the storage
   for a divergence, in whichever transcripts they were. A child that only
   this computer has was such a change.

### The rules

1. **The page leaves out the card of a call whose child came with a Pull
   without its transcript, and of no other call.** For Claude the call
   names, in its result, a child of `installed_absent_subagents`. For Codex
   the call is before `installed_end` in the current rollout and names no
   child of `installed_subagents`. These are the rules that tell the
   children of a session for Push, so a child that Push stores has a card.
   A call without a result names no child, and a call that a resumed
   session copied from an earlier one names no child of this session: both
   have their cards, as on the computer that made them.
2. **The page takes those calls out of the index it serves.** The count of
   the header, the pages of the timeline, the detail of a call and the page
   of a child then agree. The saved index holds every call, because a later
   Pull can bring the transcripts. What a Pull installed joins the identity
   of the snapshot, so a Pull ends the open cursors.
3. **`pulled_children_excluded` is retired.** Nothing reads it, and no
   Pull writes it. The binding refuses unknown keys, and at first the key
   was refused with them: the owner's own `conversation_bindings.json`
   then failed to parse, and on 2026-09-30 the owner had the key made
   optional ("Go and make them optional"). The field stays on the binding
   with `#[serde(default, skip_serializing)]`: a row of versions 0.1.3 to
   0.1.5 is read, and the key is gone at the next write of the document.
4. **A child that only this computer has stands against nothing in the
   storage when the update keeps it.** An update in place of Claude keeps
   it, by rule 2 of section 14. With a change in the storage, and a main
   transcript that was not continued here, the session is behind the
   storage. Pull installs the stored main transcript and the stored
   children, and leaves the child where it is. Afterwards the session is
   ahead by that child, and Push stores it. The session stays one session.

What stays Diverged, with Keep both:

| Case | Why |
|---|---|
| The main transcript was continued here, and the storage holds a child or a longer transcript | The Pull would replace the main transcript. A Pull that installs children and leaves the main transcript is a new action. The owner left the case to Keep both. |
| A child was continued here, and the storage changed | The Pull would replace the child. |
| A child is on both sides and nothing relates the two | As before. |
| Codex: a child that only this computer has, and the storage changed | A Codex update is written under a new identity. The child belongs to the rollout it was made under and would be left behind. |

### Code

| What | Where |
|---|---|
| `agent_calls_without_transcript`, with an empty default | `SessionAdapter`, `providers/traits.rs` |
| The rule of Claude, by the ID of the child | `providers/claude/subagents.rs` |
| The rule of Codex, by the place of the call and the installed children | `codex_subagents.rs` |
| The session keeps its binding. `leave_out_agent_calls` takes the calls out of the index, hands the tool summary of a turn to the entry that would have held it, and lowers the count. The two branches on the flag are gone | `session_detail.rs` |
| The flag's check and the three writes of it in Pull apply. The field stays, accepted and never written | `mallard-domain/src/lib.rs`, `service.rs` |
| `fold_conversation_relations` takes `local_only_children_kept` | `conversation_tail_relation.rs` |
| `ConversationTailPins.local_only_subagents_kept`, which the stale check compares | `mallard-domain/src/lib.rs`, `conversation_sync.rs` |
| `update_keeps_local_only_children`: false by default, true for Claude | `SessionAdapter`, `providers/claude/mod.rs` |
| The review gives the fold and the pins what the adapter says | `service.rs`, `provider_thread_review` |

Pull apply is unchanged but for the flag. It installed the stored children
and kept the children made here before: only the comparison stood in the
way.

### Metadata

| Metadata | Change |
|---|---|
| `conversation_bindings.json` | `pulled_children_excluded` is not written any more. A row that holds the key is read, and the key is dropped at the next write. Schema 1. |
| A reviewed plan, in memory | One required member of the tail pins, `local_only_subagents_kept`. |
| `session_timeline_cache` | Unchanged: the saved index holds every call. |
| The storage, and `src/types.ts` | Unchanged. |

### What this reverses

| Passage | Before | After |
|---|---|---|
| Section 14, "Found while testing, and not changed", the first two items | Not changed. | Changed. |
| Section 14, rule 2, the metadata table and the mutation table | `pulled_children_excluded` follows the children made here. | The flag is removed. The absent list alone says it. |
| `PLAN_CLAUDE_OWNED_SUBAGENT_SYNC.md`, "Compare uses the existing implementation" | "Add no provider parameter or second truth table." | The fold takes what the update keeps. It names no provider. The passage has a dated note. |
| `README.md`, "Child changes use the same comparison rules as main changes" | No exception. | The child that only this computer has, and the cards of the session page. |
| `CURRENT_METADATA_LAYOUT.md`, the table of local files and section 3.3 | The flag is described. | The rule of the page and the rule of the comparison are described. |
| The page of a copy that an earlier Pull installed | Its card was hidden. Its page opened by its address, and the detail of its call was served. | No card, no page, and no detail of the call: the call is not in the index that the page serves. |

### Tests

| Case | Test |
|---|---|
| Defect 1, both providers: no card after the Pull, the card of the child made here, its page, and the Push that stores it | `a_child_made_after_a_pull_without_children_has_its_card` |
| Defect 2, the Pull found a child made here | `a_reference_that_came_without_its_transcript_has_no_card` |
| Defect 2, a version that says children are stored and holds none, on a third computer. Both providers | `a_reference_of_a_version_stored_with_children_on_has_no_card` |
| Children that came with their transcripts. Both providers | `children_that_came_with_their_transcripts_have_cards` |
| A call without a result | `a_call_without_a_result_keeps_its_card` |
| A call copied from an earlier session | `a_call_copied_from_an_earlier_session_keeps_its_card` |
| A desktop import of a pulled session | `the_cards_of_a_pulled_session_are_the_same_after_a_desktop_import` |
| A copy that an earlier Pull installed | `a_copy_that_an_earlier_pull_installed_has_no_card_and_no_page`, and the two tests of section 14. The first of them is renamed `children_pulled_without_transcripts_have_no_agent_cards_until_they_are_pulled`; both assert the two installed lists where they asserted the flag |
| Keep both | `keep_both_leaves_each_session_its_cards` |
| A Codex update without children | `a_codex_update_without_children_has_no_cards` |
| Defect 3: the Pull, the Push after it, and the Pull of the first computer | `a_pull_brings_the_children_of_two_computers_together` |
| The main transcript was continued here | `a_main_transcript_continued_here_beside_stored_children_keeps_both` |
| A longer stored main transcript beside a child made here. Claude updates, Codex keeps both | `a_child_made_here_beside_a_longer_stored_session` |
| The two rules of the providers, the snapshot, the tool summary and the count | `a_call_is_left_out_by_the_id_of_the_child_it_names`, `a_call_before_the_installed_end_is_left_out_unless_its_child_was_installed`, `what_a_pull_installed_of_the_children_changes_the_viewer_snapshot`, `an_agent_call_that_is_left_out_hands_on_the_tool_summary_of_its_turn` |
| The fold | `a_child_of_this_computer_alone_stands_against_the_storage_unless_a_pull_keeps_it`, and `fold_counts_new_children_and_mixed_sides_without_hiding_invalid_evidence` for both answers |
| The plan and its stale check | `a_child_of_this_computer_alone_is_kept_by_an_update_or_by_keep_both`, `a_plan_is_stale_when_the_pull_keeps_other_children_than_reviewed` |
| The key of the flag is read and not written again | `reshaped_documents_reject_unknown_and_retired_keys` |

Each part of each rule was removed once, in the scratch copy, and a test
failed each time:

| Rule | Part that was removed | Tests that failed |
|---|---|---|
| 1 | Claude: the ID of the child in the absent list | 11 |
| 1 | Codex: the call is before the installed end | 2 |
| 1 | Codex: an installed child keeps its card | 3 |
| 1 | Codex: the place is counted in the current rollout | 1 |
| 2 | The page takes the calls out of the index | 13 |
| 2 | What a Pull installed joins the snapshot | 1 |
| 2 | The tool summary goes to another entry of the turn | 1 |
| 2 | The count of the header is lowered | 14 |
| 4 | The fold keeps the child for every provider | 3, one of them the Codex case |
| 4 | The fold keeps no child | 4 |
| 4 | The child is kept beside a main transcript on one side only | 1 |
| 4 | A child continued here counts as one of this computer alone | 3 |
| 4 | Claude does not say that its update keeps them | 2 |
| 4 | Codex says that its update keeps them | 1 |
| 4 | The review folds without the fact | 2 |
| 4 | The stale check does not compare the pin | 1 |

### Verification

On the working tree, with a target directory of its own.

| Check | Result |
|---|---|
| `cargo test --workspace --all-targets` | Passed: 1,526 tests, 0 failed, 28 ignored. Backend library 494, infrastructure 787, domain 103. |
| `cargo clippy --workspace --all-targets` | Two findings on lines of this change, both a `clone` in a test, fixed. No other finding on a line of the change. |
| `rustfmt --check` on each changed file | Each file has the count of findings it had before; the files that were clean are clean. |
| `git diff --check` | Clean. |
| `npm run test:frontend-integration` | Not run. No frontend file and no DTO changed. |
| `node scripts/run-live-claude-aimock-e2e.mjs`, with Claude Code 2.1.266 | Passed, both scenarios: `desktop-import` 19 rounds, 4 imports; `sync` push, pull and divergence. |

### Cost

Not measured, on the owner's word. The work that is new:

- **The session page.** The session keeps a copy of its binding. Each
  request walks the entries of the index once, in memory, and reads no
  file for it.
- **The comparison.** Nothing new is read. The fold takes one fact that
  the adapter says.
- **Pull and Push.** Unchanged.

### Still open

- **A main transcript continued here beside stored children.** Keep both,
  on the owner's word. One session would need a Pull that installs
  children and leaves the main transcript: a seventh plan action, with the
  review UI and `src/types.ts`.
- **Codex.** A child that only this computer has beside a change in the
  storage stays Keep both. One session would need an update that carries
  the child over to the new identity.
- **The items of section 14 that are still open.** Unchanged.
