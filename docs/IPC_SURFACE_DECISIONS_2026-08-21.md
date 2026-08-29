# IPC surface decisions — the 21 commands removed on 2026-08-21

**Status:** Landed (uncommitted). Supersedes the registration freeze test.

Round 2 of `BACKEND_OVERENGINEERING_AUDIT_2026-08-21.md` (findings A1, A2, A3)
removed 21 of the 96 registered Tauri commands. This file records the decision
behind each one — specifically whether a UI for it is planned — so the intent
lives in prose that can be argued with, instead of in a test that only says
"this name exists".

## What "dead" meant here

A command was treated as dead only when **both** directions were checked:

1. **Forward.** Its name appears in no `invoke("...")` anywhere under `src/`,
   or it has a wrapper in an `api.ts` whose only references outside its own
   definition are non-rendering stubs and specimens (`catalog-live.tsx`,
   `*-mockup.tsx`) — never a call from a component or a test.
2. **Reverse.** Every distinct `invoke("...")` name in the frontend resolves to
   a real registered command. There are **no orphan invokes**, so "unreferenced"
   cannot be an artifact of a name the backend never had.

The split depends on the baseline, and both are worth stating:

- **Against the working tree this change edited: 8 with no wrapper, 13 with a
  dead wrapper.** This is the split the removal was measured on.
- **Against the last commit (`e0d03b6`): 7 with no wrapper, 14 with a dead
  wrapper.** The difference is `get_bundle_readiness`: its `getReadiness`
  wrapper existed at `e0d03b6` and had already been deleted by unrelated
  in-flight work before this change began, so it presented as wrapper-less.

Either way the total is 21. (The audit register said 7 + 12 = 19; it missed
`get_bundle_readiness` entirely and counted `delete_catalog_resource_profile`
separately as A3.)

Reachability alone is not the argument. A command can be unreferenced because
its screen has not been built yet. Each entry below therefore says what the
live replacement is, or why no UI is coming.

## Superseded by a richer live command

The read exists; a different command already answers the same question with
more context, and the frontend uses that one.

| Removed | Live path that answers the same question |
| --- | --- |
| `get_local_project` | `get_project` — returns the registration plus links, binding, and materializations (`ProjectDetail`) |
| `get_project_binding` | `list_project_bindings` — the UI holds all bindings, not one |
| `list_remote_bundles` | `list_remote_bundle_snapshots` — snapshot summaries, not a raw cursor page |
| `find_remote_bundle_matches` | `get_sibling_bundle_status` — the fingerprint match reached the UI as sibling state instead; the helper behind it stays and is still used |
| `get_bundle_status` | **Partly** `prepare_push_review` — read the correction below before relying on this row |
| `get_bundle_readiness` | `get_restore_readiness` — the shipped readiness read is plan-scoped; both delegate to the same `bundle_readiness_for_snapshot` |
| `list_project_materializations` | `get_project` — the full `Vec<MaterializationRecord>` ships as `ProjectDetail.materializations`. Activity does *not* carry them: it reduces the records to thread-id aliases and a `last_pull_at` fallback. |
| `register_local_project` | `finalize_project_setup` — projects are created through the setup draft flow, never directly |

> **Correction.** An earlier draft of the `get_bundle_status` row claimed
> `prepare_push_review` "computes the same per-resource local/remote digest
> comparison". It does not, and the distinction matters. For a **Git-managed**
> project the review runs `build_project_files_inventory`, whose
> `ProjectContentEntry` carries `local_digest` / `storage_digest` /
> `base_digest` and classifies through `classify_project_content_state` — a
> **three-way** comparison that strictly exceeds the deleted two-way one. But
> that path is gated on `eligibility.state == GitManaged`
> (`commands.rs:7872`), and it covers project **content files**. For the
> recipe's provider resources there is no per-resource digest comparison
> anywhere in the review: `InventoryResource` carries no digest and no remote
> field. So `get_bundle_status` did compute something no surviving code
> computes. The honest argument for removing it is not coverage — it is that no
> screen ever rendered it, so nothing regressed because nothing consumed it. A
> recipe-resource drift view, if ever wanted, should be rebuilt against the
> review's inventory rather than as a second command.

No UI is planned for the rest: building one would mean showing the same facts
twice, from two code paths that could disagree.

## Decided against a UI

| Removed | Decision |
| --- | --- |
| `rename_provider_profile`, `remove_provider_profile` | A provider profile is a saved provider home, created on demand and keyed by path (`existing ?? createProviderProfile`). It behaves as a cache, not a user-managed object. `tests/frontend/storage-catalog.integration.test.tsx` already asserted the Add-resource screen must not grow "Manage saved homes" controls. **Known gap:** there is now no way to remove a stale profile at all; the intended fix is automatic pruning of unreferenced profiles, not a management screen. |
| `delete_catalog_resource_profile` (A3) | Same decision on the Library side. The same test asserted the UI must not reference `api.deleteProfile` — a command whose own test forbade its UI. |
| `remove_project_binding` | Detach had no caller. A binding is changed by saving a new one (`save_project_binding`); removing the project (`remove_local_project`) is the coarse exit. A detach-without-remove flow is not planned. |
| `save_bundle_recipe` | The project recipe is machine-local selection state. As `build_resource_inventory` puts it, a reviewed Push is the only flow that changes a destination recipe. Connecting to a remote bundle seeds it; nothing else should write it directly. |
| `discard_restore_plan` | Restore plans expire on their own (`restore_support_context` enforces the window). An explicit discard button was never built and would only race the expiry. |
| `verify_bundle_objects` (A2) | A full download-and-rehash durability audit of every object in a manifest, with no test, no TS type, and no screen. Publish verifies by upload receipt (each PUT carries its sha256); this was the read-back Push deliberately does not perform. If durability auditing is wanted it should be a CLI subcommand with progress and cancellation, not a Tauri command that blocks on the whole manifest. |

## The origin-host-alias slice

Six commands — `update_host_display_name`, `rotate_host_identity`,
`get_project_origin_tokens`, `list_origin_host_aliases`,
`save_origin_host_alias`, `resolve_origin_host_label` — formed a vertical
slice with no screen at any layer.

Its last non-test caller was `ThreadSyncEntry.origin_label` (register finding
B22), computed in `provider_thread_review` for every **remote-present thread
carrying a recorded origin** — the `.and_then` short-circuited local-only rows,
so a storage-less Activity fill made no such calls. For the rows that did
qualify, each call took a cross-process persistence lock and parsed the whole
conversation-bindings document. No component read the field: `types.ts`
declared it and only a mockup fixture ever set it, to `null`. The block also
carried a per-thread `.expect("stored conversation requires a bundle snapshot")`,
so removing it retires a panic path as well. That computation is gone.

**Not planned:** a device-rename or device-rotate screen. Host identity
auto-registers on app open, and its `local_display_name` is never set — the only
writer, `Repository::update_host_display_name`, has no production caller, so
`auto_register_host_identity_unlocked` stores `None` and every reader falls
through to the token-derived `Host ABCD` label. `get_host_identity_status`
(live) is the only read the UI needs.

**Not planned:** user-visible aliases for other hosts. The shipped
provenance marker uses `fallback_origin_host_label` directly — a bounded
`Host ABCD` label derived from the token — which is the privacy-preserving
default the alias layer was going to let users override.

### What deliberately stayed

`Repository::resolve_origin_host_label`, `mutate_origin_aliases`,
`update_host_display_name`, and `rotate_host_identity` still exist with only
test callers. They are the sole readers and writers of `ConversationBindings.origin_aliases`
and of the host identity document. `ConversationBindings` is
`#[serde(deny_unknown_fields)]`, so dropping the field is a local-state
migration — every existing `conversation_bindings.json` would fail to parse —
not dead-code removal. That is a wipe-based cutover and belongs in its own
change.

`derive_project_origin_tokens` and `load_origin_aliases` had no callers at all
and were deleted. `derive_origin_tokens` is live: it derives the per-bundle,
non-correlatable tokens that publishing stamps into `published_by_origin`, and
the privacy properties around it stay under test.

## Test-only code created by this change

Two helpers now live inside `mod tests` because their production entry point is
gone but they set up or observe state for tests that are about live behavior:

- `register_local_project_with_repository` (with its request struct, serde
  derives dropped) — the shortest way to make a project exist. Nine tests call
  it directly; the rest go through the shared `register()` helper, so roughly
  sixty depend on it transitively. **Drift risk worth naming:** production
  creates projects through the setup draft flow, and the fixture already differs
  from it in three ways — it passes `None` as the `default_local_alias`
  qualifier where the live path passes the bound profile's display name; it
  always starts from `BundleRecipe::default()` where the live Existing-repo
  branch seeds from `machine_local_recipe_from_manifest`; and it always
  generates a fresh id where the live path may reuse a detached binding's.
  Those three are the checklist for rewriting the fixture onto
  `finalize_project_setup_with_repository`.
- `readiness_for_binding` — resolves the binding and snapshot itself and calls
  the live `bundle_readiness_for_snapshot`, replacing the deleted bundle-scoped
  readiness command.

Deleted along with the code they were the only reason to keep:
`recipe_updates_require_the_opened_revision`,
`profile_removal_is_blocked_only_while_an_active_project_uses_it`,
`deleting_a_profile_refuses_while_a_scope_names_it`, one line of
`host_sensitive_workflows_share_the_host_operation_lock` (the detach arm), and
the save-side half of `machine_local_recipe_write_boundaries_reject_portable_conversations`
— whose push-side half still pins the same invariant, as do four other
production `validate_machine_local()` call sites.

## On removing the freeze test

`src-tauri/src/sync_tests/registration_freeze.rs` pinned the registration list
for `PLAN_PROJECT_STRUCTURE_REORGANIZATION.md` §2, so that a file move could
never silently change the IPC contract. It did that job. It also meant CI
actively defended 21 commands nothing could call, and the only way to delete
them was to edit the freeze — which the plan's §3.3 calls a stop-the-line
signal.

A snapshot test cannot tell a deletion from an accident, because it holds no
reason for any entry. This file holds the reasons. If the remaining reorg
phases want a guard again, the useful one is narrower: assert that every
`invoke("...")` in `src/` resolves to a registered command (the reverse
direction, which has always been clean), rather than freezing the forward list.

## Verified after landing

An adversarial pass — five independent reviewers plus a critic, each told to
refute rather than confirm — re-derived the analysis. It confirmed the IPC
surface reconciles exactly (75 registered, 75 distinct `invoke` names, zero
orphans in either direction) and found no production behavior break. It also
found four things this pass got wrong, all fixed:

1. **A real coverage regression.** Deleting
   `deleting_a_profile_refuses_while_a_scope_names_it` also removed the only
   assertion that `delete_catalog_scope_with_repositories` ever *succeeds* — and
   `delete_catalog_scope` is live and UI-reachable. Its happy path, its
   `expected_revision` guard, and its unknown-scope `Ok(false)` branch were left
   untested. Restored as
   `deleting_an_unowned_scope_removes_the_row_and_pins_its_revision`. The claim
   above that the deleted tests covered only deleted code was wrong for this
   one: its second half tested live code.
2. **A lost guard assertion.** `detach_project_binding_guarded` is still live via
   `remove_local_project_guarded`, so dropping the detach arm from
   `host_sensitive_workflows_share_the_host_operation_lock` left that mutation's
   host-lock behavior unpinned. The arm now names
   `remove_local_project_with_repository`.
3. **A guard weakened for no reason.** The `api\.deleteProfile` term was dropped
   from the frontend `doesNotMatch` assertion that this document cites as its
   justification for removing that command. It would have passed as written;
   restored.
4. **Two missed orphans.** `LocalStateRepository::discard_restore_plan` had zero
   references anywhere — test or production — and `ProjectResourceState` in
   `types.ts` lost both of its users when this change deleted
   `BundleResourceStatus` and `ResourceStatusReport`. Both deleted.

Three further items are recorded rather than actioned:

- **Restore plans are never swept.** With `discard_restore_plan` gone, nothing
  deletes an individual plan file; only the two bulk resets do.
  `restore_support_context` rejects an expired plan at use but leaves it on disk,
  so `~/.mallard/restore_plans/` accumulates. "Restore plans expire on their
  own" above describes validity, not lifecycle.
- **The kept alias methods should be `#[cfg(test)]`-gated.** Keeping
  `origin_aliases` on disk does not require its four accessors to compile into
  production. Gating them — and the two private helpers only they reach,
  `load_conversation_bindings_for_aliases_unlocked` and
  `validate_origin_aliases_unlocked` — would let the compiler enforce what this
  document currently only asserts in prose. The precedent sits ten lines away:
  `#[cfg(test)] pub fn derive_origin_host_token`.
- **`replace_host_identity_unlocked`'s strict branch is production-unreachable.**
  Its only `current: Some(_)` caller is `rotate_host_identity`, now test-only, so
  production always enters with `allow_damaged_recovery == true` and only the
  quarantine arms can run — including the one that substitutes
  `ConversationBindings::empty(...)` on a parse failure. That sharpens the
  migration warning above: removing `origin_aliases` later would not merely fail
  to parse, it would quarantine and discard every conversation binding.

**Knowingly stale docs.** `DESIGN_PUSH_PULL_FLOW.md` and
`DESIGN_FAST_PUSH_PULL.md` still describe `verify_bundle_objects` as a shipped
command, and `PLAN_PROJECT_STRUCTURE_REORGANIZATION.md` (§10-§12) still names
the deleted freeze test as a per-PR acceptance gate. Those are records for work
still in flight; amending them belongs with whoever closes out the
reorganization, and the reverse-direction guard proposed above is what should
replace the freeze in its acceptance list.
