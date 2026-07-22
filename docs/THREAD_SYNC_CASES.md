# Thread Sync Cases

This document defines Push and Pull behavior for one linear chat thread.
Each letter represents a message, and message identity and parentage—not display
text—determine whether one version extends another.

## Default policy

- Never remove a version by default.
- Fast-forward when one version is an exact prefix of the other.
- When versions diverge, keep the current primary and add the other as an
  alternate.
- Changing the primary version requires explicit approval.
- Replaced versions remain available in immutable history or a local backup.

## Complete matrix

| Relationship | Push | Pull |
| --- | --- | --- |
| Local and cloud are identical | No-op | No-op |
| Cloud is a prefix of local | Fast-forward cloud | Keep local; no-op |
| Local is a prefix of cloud | Block; Pull required | Fast-forward local |
| Both extend a common message differently | Add local as a cloud alternate | Add cloud as a local alternate |
| No common ancestor is known | Treat as diverged | Treat as diverged |

## Push cases

### Diverged

```text
Local: [a, b, c1]
Cloud: [a, b, c0]
```

Default—keep both:

```text
Cloud primary:   [a, b, c0]
Cloud alternate: [a, b, c1]
Local primary:   [a, b, c1]
```

Explicit—make local primary:

```text
Cloud primary: [a, b, c1]
Cloud history: [a, b, c0]
Local primary: [a, b, c1]
```

The explicit action requires approval and a compare-and-swap check against the
reviewed cloud version.

### Local ahead

```text
Before: Local [a, b, c1]  Cloud [a, b]
After:  Local [a, b, c1]  Cloud [a, b, c1]
```

This is a safe fast-forward Push.

### Cloud ahead

```text
Local: [a, b, c1]
Cloud: [a, b, c1, c2]
```

Push makes no changes and directs the user to Pull. Pull can then fast-forward
the local thread.

### Identical

```text
Local: [a, b, c1]
Cloud: [a, b, c1]
```

No operation is needed.

## Pull cases

### Diverged

```text
Local: [a, b, c2]
Cloud: [a, b, c0]
```

Default—keep both:

```text
Local primary:   [a, b, c2]
Local alternate: [a, b, c0]
```

Explicit—make cloud primary locally:

```text
Local primary: [a, b, c0]
Local backup:  [a, b, c2]
```

The explicit action requires approval and must verify that the local version
has not changed since review.

### Cloud ahead

```text
Before: Local [a, b]      Cloud [a, b, c0]
After:  Local [a, b, c0]  Cloud [a, b, c0]
```

This is a safe fast-forward Pull.

### Local ahead

```text
Local: [a, b, c2]
Cloud: [a, b]
```

Pull is a no-op. The thread remains local-ahead and can later be pushed safely.

### Identical

```text
Local: [a, b, c2]
Cloud: [a, b, c2]
```

No operation is needed.

## Missing and deleted threads

Absence must not implicitly mean deletion.

| Local | Cloud | Default behavior |
| --- | --- | --- |
| Missing | Present | Pull creates the local thread |
| Present | Missing | Push creates the cloud thread; Pull keeps local |
| Missing | Missing | No-op |

Deleting a thread everywhere requires an explicit tombstone. A delete/modify
race is treated as divergence and preserves the surviving version.

## Required identity model

Every message needs a stable `message_id` and `parent_message_id`. Every
alternate needs a unique `variant_id` linked by a shared `conflict_group_id`.
If alternates are materialized as provider-visible threads, each must receive a
unique thread/session ID.
