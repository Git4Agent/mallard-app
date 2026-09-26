# Preserve Unfamiliar Codex Records: Minimal Plan

Date: 2026-09-22

Status: Implemented. Verification results are recorded below.

## Objective

Let Mallard carry unfamiliar Codex items and events as JSON records through
Push and Pull. Session-detail rendering support can be added separately.

Preserve the existing path mapping, credential redaction, integrity checks,
and reviewed restore process.

## Problem before the fix

The projector in
`crates/mallard-infrastructure/src/codex_paginated_projection.rs` rejected:

- An `event_msg` with an unfamiliar payload type.
- An `event_msg/item_completed` whose item type is absent from `ITEM_KINDS`.

Codex 0.156.0 can persist the completed item type `FunctionCallOutput`.
Mallard rejected this item during conversation publication. Other selected
conversations could still publish.

This is different from `response_item/function_call_output`, which already
travels through the existing record path. Extra fields in accepted completed
items are also already preserved.

Adding each new item type to the allowlist would leave the same failure for
the next unfamiliar type.

## Minimal production change

Change only `codex_paginated_projection.rs`.

### Completed items

- Remove the fixed item-type allowlist.
- Require the item type to be a nonempty string.
- Keep the existing payload-object, item-object, item ID, and turn ID checks,
  including the current identifier length limits.
- Preserve the complete item payload and extra fields.
- Continue normalizing the enclosing `thread_id` to the output identity.

### Events

- Require the event payload to be an object with a nonempty string type.
- Preserve an unfamiliar event as a source JSON record, in order.
- Do not generate a display item or change turn state for that event.
- Retain the current conversion and validation rules for recognized events.
- Remove the context-event allowlist if the generic preservation branch
  makes it redundant.

Update the module comments to state that transport does not require rendering
support for every item or event type.

## Processing boundary

Unfamiliar records must use the existing bundle processing. They must not
bypass path mapping, configured credential redaction, source verification,
digest verification, or reviewed restore checks.

Preservation means preservation of JSON content subject to the existing path,
identity, and redaction transformations. It does not mean a byte-for-byte copy
of the original file.

This change removes rejection based only on an unfamiliar type name. It does
not infer the meaning of new fields. In particular, it does not add identity
rewriting rules for new reference fields or guarantee that an older Codex
version can render a newer record.

## Regression tests

Add tests to the two existing files below. Use synthetic records and temporary
directories. Do not use real provider homes.

### `codex_paginated_projection.rs`

- Preserve a completed `FunctionCallOutput` item.
- Preserve a made-up future completed item type.
- Preserve an unfamiliar event and nested extra fields.
- Verify record order and completed-item thread identity normalization.
- Verify that another projection pass adds or changes nothing.
- Verify that malformed envelopes, missing or empty types, and invalid item
  or turn IDs still fail.
- Retain the existing tests for recognized-event conversion.

### `codex_standalone_builder.rs`

Add an encode → restore → encode test with unfamiliar items and events.
Verify that:

- Their JSON content and order survive the round trip.
- The restored completed-item envelope uses the destination thread identity.
- Nested project paths use the destination root after restore.
- Synthetic credentials in unfamiliar fields still undergo redaction.
- Re-encoding preserves the portable content under the existing identity and
  path rules.

## Validation

Run the focused projector and standalone-builder tests while implementing.
Then run:

```sh
cargo test -p mallard-infrastructure --lib
cargo test --manifest-path src-tauri/Cargo.toml --lib
```

These checks establish Mallard preservation and restore behavior. They do not
establish native Codex rendering support for arbitrary future types.

## Implementation results

`FunctionCallOutput` was first added to the existing item list. Its focused
test passed with text and structured output, extra fields, and thread identity
normalization. The generic preservation change then removed both type lists.
The `FunctionCallOutput` regression test remains.

The production change is confined to the projector. The standalone builder
has one new round-trip test. Session-detail rendering is unchanged.

Validation completed:

- Focused Codex tests: 99 passed.
- Infrastructure library: 661 passed, 5 ignored, 0 failed.
- Tauri library: 378 passed, 6 ignored, 0 failed.
- Formatting checks on both Rust files and `git diff --check`: passed.

Native Codex and paid-provider E2E tests were not run for this change.

## Scope

The implementation changes the projector and adds tests in the standalone
builder. It requires no schema change, migration, dependency, or UI change.

Keep current session-detail handling and warnings. New renderers, the separate
`AgentMessage` delivery/questions conversion gap, and CLI test-version pins
are outside this fix.

## Acceptance criteria

A structurally valid unfamiliar record no longer blocks a conversation solely
because its type is unknown. Its content survives Mallard's encode and restore
path, with existing safety transformations applied. Malformed records still
fail the applicable validation. Recognized records retain their current
behavior.
