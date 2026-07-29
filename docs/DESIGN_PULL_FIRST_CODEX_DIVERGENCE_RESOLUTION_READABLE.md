# Pull-first Codex Conflict Handling — Readable Version

- Status: Approved direction; implementation pending
- Cutover assumption: there are no existing users whose Mallard state must be
  preserved
- Companion technical design:
  [DESIGN_PULL_FIRST_CODEX_DIVERGENCE_RESOLUTION.md](DESIGN_PULL_FIRST_CODEX_DIVERGENCE_RESOLUTION.md)

## Main promise

**Mallard will not resolve a conversation conflict during Push.**

If the local and cloud conversations both changed, Push stops and asks the
user to Pull first. Pull then lets the user keep both conversations or
explicitly discard the local one.

The safe default is always:

> Preserve the local conversation as a real Codex fork, then restore the cloud
> conversation into the original local thread.

## Example

Local and cloud share two messages, then diverge:

```text
local:  m1 → m2 → m3
cloud:  m1 → m2 → m4
```

After the recommended Pull:

```text
original Codex thread: m1 → m2 → m4
new Codex fork:        m1 → m2 → m3
cloud:                 m1 → m2 → m4
```

Nothing is uploaded during Pull. The cloud conversation stays exactly as it
was.

## What happens during Push

For a conflict, Push shows:

```text
Pull required
Local and stored history have diverged. Pull and choose how to preserve the
local branch before pushing.
```

The row is not selectable. Push cannot:

- overwrite the cloud conversation;
- upload the local branch under a surprise identity;
- create a conflict fork; or
- ask the user to force the Push.

After a successful Pull, a later normal Push may upload the preserved local
fork as a separate cloud conversation.

## What the user chooses during Pull

Pull shows two choices:

### Keep both branches — Recommended

Mallard:

1. asks Codex to fork the current local conversation;
2. verifies that the fork contains the complete local conversation;
3. gives the fork a predictable name that marks it as the pre-Pull local
   branch;
4. keeps that fork as a separate local conversation;
5. backs up the original conversation;
6. restores the cloud conversation into the original Codex thread;
7. applies the cloud conversation title to the original thread when one is
   available; and
8. leaves the cloud untouched.

The user can continue either conversation afterward.

### Use cloud version only

Mallard replaces the original local conversation with the cloud version.
It also applies the cloud conversation title when one is available.

The divergent local branch remains only in Mallard's private recovery backup;
it is not available as a normal Codex conversation. Because of that loss, this
choice requires an explicit destructive confirmation.

Mallard never switches to this choice automatically when fork creation fails.

## How Mallard creates the Codex fork

Mallard tries three methods in order.

### 1. Codex app-server

Mallard first asks the installed Codex CLI to create the fork through its
machine-readable app-server interface.

This is preferred because it does not open an interactive screen, type into a
composer, or change the currently focused Codex App conversation.

### 2. Hidden PTY helper

If app-server is definitely unavailable and definitely created no fork,
Mallard runs `codex fork` inside a controlled pseudo-terminal.

The helper:

- does not show the conversation transcript;
- does not submit a prompt;
- waits for Codex to save the new fork;
- exits the Codex terminal screen; and
- returns the new thread ID to Mallard.

The current helper prototype supports macOS and Linux. Native Windows requires
an equivalent ConPTY helper. Until that is implemented and tested, Windows
moves directly from app-server to the manual method.

### 3. Ask the user to fork

If neither automated method is safely available, Mallard pauses and asks the
user to fork the exact conversation in Codex App or CLI.

The user returns to Mallard and selects **Verify fork**. Mallard then finds or
accepts the new thread ID and performs the same checks used for an automated
fork. Mallard shows the suggested fork name at this checkpoint. A name the
user explicitly chose is preserved.

Canceling this step leaves the original conversation unchanged.

## Mallard does not blindly retry

A failed command does not always mean that no fork was created. Codex may have
saved the fork just before Mallard lost the response.

Mallard therefore follows this rule:

| What Mallard can prove | What happens |
| --- | --- |
| No fork command ran | try the next method |
| The command was rejected and no fork exists | try the next method |
| The command may have created a fork | stop and inspect the result |
| Exactly one correct fork exists | adopt it and continue |
| The result changed or multiple forks exist | stop for recovery |

This prevents Mallard from creating several duplicate forks while trying
different methods.

## How Mallard verifies the fork

Before replacing the original conversation, Mallard confirms that the new
fork:

- has a different Codex thread ID;
- identifies the original thread as its direct parent;
- contains the same messages in the same order as the reviewed local branch;
- contains no added prompt, model response, or interruption marker;
- belongs to the expected Codex profile; and
- was not already present before Pull began.

Mallard also checks that the original local conversation and cloud version
have not changed since the user reviewed the Pull.

If any check fails, the original conversation is not overwritten.

Conversation titles are not used for any of these checks. Codex thread IDs and
verified content determine identity; titles are only labels.

## How the conversations are named

Mallard shows the planned names during Pull review and keeps them fixed across
retries.

The preserved local fork is named:

```text
<current local title> — local before Pull · <Pull plan time in UTC>
```

For example:

```text
Analyze codebase knowledge graph — local before Pull · 2026-07-29 18:42Z
```

Mallard chooses the base title in this order:

1. the explicit local Codex thread name seen during review;
2. the cloud conversation title;
3. a short summary of the first local user message; or
4. `Untitled conversation`.

The restored original thread receives the cloud conversation title when the
cloud has one. If the cloud has no title, Mallard keeps the original thread's
pre-Pull title.

Names are cleaned and length-limited. They never contain machine names, local
paths, provider IDs, Mallard logical IDs, or cloud storage locations.

Mallard asks Codex to save these names only after the corresponding content is
verified. If naming is unavailable or fails, Pull still preserves the correct
content and identities. The receipt reports the failure and the suggested
name, so the user can rename the thread later. Mallard never edits Codex index
or database files merely to force a title change.

## What happens in Codex App

Creating a valid Codex thread and refreshing a running Codex App are separate
things. The new fork may not immediately appear in the expected App project.
When Codex naming support is available, the fork has the distinct planned
pre-Pull name and the restored original uses the cloud title when one exists.

Mallard does not:

- open the new thread;
- change App focus;
- use a deep link;
- use private App communication; or
- force-restart the App.

After Pull is completely committed, Mallard may offer:

```text
Restart Codex App
```

Restart is optional. The user must first confirm that active chats have
finished. Declining or failing the restart does not affect the completed Pull.
If the fork is not visible under the expected project, the user can check the
App's chronological conversation view.

## Safe failure behavior

Before Mallard changes the original conversation:

- the cloud replacement is prepared but not installed;
- the native fork must be fully saved and verified;
- the local and cloud versions are checked again;
- the source conversation must not be actively changing; and
- Mallard records enough recovery state to resume safely after a crash.

If Mallard crashes after Codex creates the fork, it searches for that exact
fork instead of creating another one.

If Mallard crashes after replacing the original conversation, it verifies the
saved result and finishes the local bookkeeping. It never repeats the fork.

Uncertain extra Codex threads are never deleted automatically.

## Final local state

After **Keep both branches**:

| Conversation | Content | Name |
| --- | --- | --- |
| Original Codex thread | cloud branch | cloud title, or unchanged when cloud has no title |
| New Codex fork | previous local branch | planned `local before Pull` name |

After **Use cloud version only**:

| Conversation | Content | Name |
| --- | --- | --- |
| Original Codex thread | cloud branch | cloud title, or unchanged when cloud has no title |

In both cases, Pull leaves cloud storage unchanged.

## Clean cutover

This is a replacement for the current conflict behavior, not an additional
compatibility mode. There are no deployed users or existing Mallard records
that this change must preserve.

- Old conflict plans are not accepted.
- Old conflict journals are not recovered.
- The previous Push-side fork behavior is removed.
- The previous Pull rekey-and-copy behavior is removed.
- No migration, compatibility reader or writer, feature flag, or legacy
  fallback is added.
- Unsupported stale state is rejected instead of upgraded.

Before the next manual acceptance run, the owner will clear `~/.mallard` and
the designated test cloud buckets. Mallard will not automatically delete that
state. Automated tests continue to use temporary Mallard homes and disposable
cloud fixtures rather than real user or production state.

## Done when

- A conversation conflict always blocks Push.
- Pull defaults to keeping both branches.
- Fork creation tries app-server, then PTY, then manual verification.
- Mallard never tries the next method after an uncertain fork attempt.
- The original Codex thread contains the cloud conversation after Pull.
- The new Codex fork contains the exact previous local conversation.
- The fork receives the planned distinct pre-Pull name when Codex naming is
  available.
- The original receives the cloud title when one exists; otherwise its title
  remains unchanged.
- A naming failure is reported without undoing a content-safe Pull.
- Pull never modifies cloud storage.
- Cloud-only always requires destructive confirmation.
- Crashes and retries create no duplicate adopted forks.
- App restart remains optional and happens only after Pull is safe.
- Only the new conflict representation and execution path remain.
