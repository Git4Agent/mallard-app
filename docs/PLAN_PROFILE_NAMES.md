# PLAN: Custom names for local and cloud profiles

Status: implemented (2026-07-15). Tests: `local_profile_label_prefers_custom_name`
(unit) and S34 `rename_propagates_and_survives_push` (both backends).

REVISED same day after live use: the user renamed a profile and expected the
cloud label to follow on push — two separate names (local field + a cloud
rename control in link settings) didn't match their model. Now ONE name: the
local profile's `name`, when set, is pushed into `_tag.json.label` on every
push (see §2 revision below); the separate `rename_cloud_profile` command and
its link-settings UI were deleted.

## Problem

Neither side of a link has a user-settable name:

- **Local profiles** have no name field at all (`LocalProfile`, `lib.rs` —
  the doc comment says "Display name derives from the path"). The sidebar and
  settings show `~/.claude` or `myconf2/.claude`; two mounts under similarly
  named folders are hard to tell apart, and nothing lets the user call one
  "Work" and the other "Personal".
- **Cloud profiles** already have a label (`_tag.json.label`, shown in the
  picker as `Claude (d0908185)`), but it is assigned once at auto-create
  (`unique_profile_label`) and never editable. Worse, every push rewrites the
  tag from the pusher's *cached* copy (`link.cloud.profile_label` →
  `write_tag_best_effort`), so even if machine A changed the tag, machine B's
  next push would silently revert it.

Requirement: each profile can carry a custom name; the cloud profile's name
lives in the cloud (any machine sees it), the local profile's name lives in
local config.

## Design

### 1. Local profile name — config field, display fallback

- `LocalProfile` gains `#[serde(default)] name: String` (`lib.rs`) and
  `name?: string` (`types.ts`). Empty = unnamed, exactly today's behavior; no
  config migration (serde default).
- Display is already centralized on both sides; each gets a one-line
  fallback:
  - `local_profile_label` (`lib.rs`) → return `profile.name` when non-empty,
    else the current path-derived label. Feeds `ConfigSource.label`, so the
    sidebar/Files view follow for free.
  - `profileLabel` (`SyncPanel.tsx`) → same. Feeds every settings-page,
    dialog, and holder-tag usage for free.
- When a name is set, keep the path visible as secondary text where the row
  already shows one (settings profile card shows the mount path today) — the
  sidebar stays honest about what syncs.
- Edit UI: a "Name" text field in the existing profile edit dialog (gear on
  the profile row), placeholder = the derived label. Saved through the
  existing `save_sync_config` dirty-state flow — no new command.
- The name never syncs as a file, but push copies it into the cloud tag
  (§2) so other machines see it on their next probe/link. Pull never
  renames a local profile — names only flow local → cloud.

### 2. Cloud profile name — the local name, pushed into `_tag.json.label`
### (REVISED: one name, no separate cloud rename)

`_tag.json` is already the designed home for display data (DESIGN2: display
cache, best-effort, outside the CAS surface). Reuse it; do not touch
`HeadFile` — putting a name in the head would make rename a CAS publish that
races real pushes, for zero safety gain on a display string.

- **Push propagates the name** (`push_profile`, `lib.rs`): when the pushing
  local profile has a non-empty `name`, `write_tag_best_effort` writes it as
  the tag label (`rename_to` param) — the cloud profile is renamed for every
  machine on the next push. No separate rename command or UI; the original
  `rename_cloud_profile` command was built and then deleted in the same-day
  revision.
- **No-change pushes rename too** (`rename_tag_best_effort`): the second
  live bug — rename then push with zero changed files hit the
  everything-up-to-date early exit before any tag write, so nothing
  happened. That path now rewrites just the tag's label (and adopts) when
  `rename_to` differs; no revision is published — the tag is display data
  outside the CAS surface.
- **Label precedence in `write_tag_best_effort`**: `rename_to` (pusher's
  custom name) > existing tag's non-empty label (one extra GET per push;
  keeps renames from other machines alive) > caller's cached
  `link.cloud.profile_label` (creation path — no tag exists yet).
- **Adoption**: the effective label is returned to the push path; when it
  differs from the link's cached copy, `adopt_profile_label` rewrites every
  saved link pointing at `(storage, profile_id)` — the renamer's config
  updates on its own push, other machines self-heal on theirs.
- Concurrency: last-writer-wins on the tag, same as today. Identity is
  `profile_id`; the label is display-only, so a lost rename is a cosmetic
  race, not corruption. Siblings sharing one cloud profile under different
  custom names flip the label per push (ponytail comment at the
  `rename_to` computation names this ceiling).
- Uniqueness is NOT enforced (`unique_profile_label` stays creation-only).
  The picker already prints the short id next to the label, which is what
  actually disambiguates.

### 3. Stop matching cloud state by label

`cloudFor` (`SyncPanel.tsx`) matches `clouds` rows by
`(storage, profile_label)` — already flagged wrong when labels collide, and
untenable once labels are mutable. Add `profile_id` to `CloudRootState`
(`lib.rs` + `types.ts`; the producing `get_file_statuses` has it in hand) and
match by `(storage, profile_id)`. Everything else already keys by id.

## Display resolution summary

| Surface | Name shown | Source |
|---|---|---|
| Sidebar / Files view (local profile) | `name` else `~/{root}` / path | `ConfigSource.label` via `local_profile_label` |
| Settings profile card, dialogs | same | `profileLabel` (TS) |
| Picker rows, link card | tag `label` | `list_sync_profiles` probe (live) |
| Link line / footer when offline | `link.cloud.profile_label` | cached copy, heals on push |

## Tests

- Unit: `local_profile_label` name fallback (custom name wins, trimmed;
  empty falls back to the derived label).
- S34 (both backends): machine A names its local profile and pushes — tag
  label and A's saved link become the name; machine B (no custom name)
  pushes — the tag keeps A's name and B's saved link adopts it.
- S35 (both backends, the live bug repro): one machine, two local profiles
  sharing one cloud profile; renaming one and pushing WITH NO FILE CHANGES
  (asserted: revision does not bump) renames the tag label (what the
  storage rows display) and heals BOTH links' cached labels; the unnamed
  sibling's own push doesn't revert. Negative-checked against both the
  publish-path and no-change-path propagation.

## Non-goals

- Migration or backups: `name` is `serde(default)` (old configs read
  unchanged), the tag is overwritten in place. Explicitly per user.
- Name uniqueness or validation beyond trimming (empty string = clear name,
  which stops propagating but never reverts the cloud label).
- Renaming the cloud *prefix* (`profile_id`) — that is identity, covered by
  pin/re-pick flows in PLAN_LINK_PROFILE_PICKER.md.
