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
   or it has a wrapper in an `api.ts` whose key has **zero** references in
   `src/` and `tests/` — the wrapper's own definition being its only mention.
2. **Reverse.** Every distinct `invoke("...")` name in the frontend resolves to
   a real registered command. There are **no orphan invokes**, so "unreferenced"
   cannot be an artifact of a name the backend never had.

The measured split at the time of removal: 8 commands with no wrapper at all,
13 with a wrapper and no call site. (The audit register said 7 + 12 = 19; it
missed `get_bundle_readiness`, which is in the same category, and counted
`delete_catalog_resource_profile` separately as A3.)

Reachability alone is not the argument. A command can be unreferenced because
its screen has not been built yet. Each entry below therefore says what the
live replacement is, or why no UI is coming.

## Superseded by a richer live command

The read exists; a different command already answers the same question with
more context, and the frontend uses that one.

| Removed | Live path that answers the same question |
| --- | --- |
| `get_local_project` | `get_project` — returns the registration plus links, binding, and sibling state |
| `get_project_binding` | `list_project_bindings` — the UI holds all bindings, not one |
| `list_remote_bundles` | `list_remote_bundle_snapshots` — snapshot summaries, not a raw cursor page |
| `find_remote_bundle_matches` | `get_sibling_bundle_status` — the fingerprint match reached the UI as sibling state instead; the helper behind it stays and is still used |
| `get_bundle_status` | `prepare_push_review` — the review computes the same per-resource local/remote digest comparison and much more |
| `get_bundle_readiness` | `get_restore_readiness` — the shipped readiness read is plan-scoped; both delegate to the same `bundle_readiness_for_snapshot` |
| `list_project_materializations` | `get_project_activity` — materializations reach the UI as activity rows |
| `register_local_project` | `finalize_project_setup` — projects are created through the setup draft flow, never directly |

No UI is planned for any of these: building one would mean showing the same
facts twice, from two code paths that could disagree.

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
B22), computed in `provider_thread_review` for **every** thread on every Push
review, Pull plan, and Activity load. Each call took a cross-process
persistence lock and parsed the whole conversation-bindings document. No
component read the field: `types.ts` declared it and only a mockup fixture ever
set it, to `null`. That computation is gone.

**Not planned:** a device-rename or device-rotate screen. Host identity
auto-registers on app open and its display name is derived; `get_host_identity_status`
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
  derives dropped) — the shortest way to make a project exist. 20 tests use it
  as a fixture. **Drift risk worth naming:** production creates projects through
  the setup draft flow, so these tests will not notice if that flow starts
  producing a different registration. Rewriting the fixture onto
  `finalize_project_setup_with_repository` is the follow-up.
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
