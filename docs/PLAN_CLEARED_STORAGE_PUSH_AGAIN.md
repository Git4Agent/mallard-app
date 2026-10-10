# Mallard: push again instead of reset — implementation plan

> Historical reference: The contents of `mockups/` and `poc/` were removed
> on 2026-10-09. Paths and commands below that use those files are historical
> and no longer work in this checkout. Git history retains the tracked files.

- Status: Implemented and committed: steps 1 to 10 on September 20, steps 11 to 14 on September 22. Step 11 fixed a gap found at review on September 22: after another computer's clear, the project's files read "unknown" and held the Push (section 4.7). Steps 12 and 13 applied the decision of September 22 (CAS-free plan, decision 12: recovery is a user action), which withdrew two built behaviors, the rollback of R3 and the Library reset at load of B1 and B2; both are the read rule of section 4. None of steps 11 to 14 needed the claims of the CAS-free plan. The live two-computer pass on R2 and on Dropbox is still to be run and recorded. *Implementation notes* at the end list what differs from the plan
- Last reviewed: September 22, 2026, against the working tree
- Scope: the desktop app — how Push and Pull behave when this computer's sync record cannot be used, and when two computers race
- Compatibility: clean cutover; no migration, no legacy detection, no legacy execution path
- Deployment assumption: there are no customers yet; the owner clears `~/.mallard` and the cloud data before retesting
- Schemas: every schema and layout number stays `1`. No persisted document changes shape
- Related: `DESIGN_CAS_FREE_STORAGE_REFACTOR.md` (sections 7.1, 7.2, 9.1, 9.2, 9.4, 11 and 12), `DESIGN_REMOTE_HISTORY_CONTINUITY.md` (its explicit-unlink rule is replaced by this plan)

Code locations are collected in *Source pointers* at the end, not in the body.

## 1. The rules

Mallard writes down, on this computer, what it last synced with a storage. It
keeps one such **record** per storage area, the *base*, and one per session,
the *acceptance*. A record says two things: what the content was, as a count
and a digest, and in which version of the storage that was agreed.

The record is what lets Mallard tell who changed since. When only the local
copy differs from the record, this computer changed it. When only the stored
copy differs, another computer did. When both differ, both did.

A storage's history is a chain of versions, and each version names the one
before it. A record is **usable** while the version it names is on that chain.
Whatever the storage holds now was then built on top of what this computer last
saw, so comparing against the record is sound.

A record is **stale** when the version it names is no longer on the chain. That
happens when another computer cleared the storage: the old chain is gone, and a
new one may have started. The record then describes a history the storage no
longer has. A record can also be **missing**: the session was never synced
here, or the record was reset by Clear storage or by Reset local sync state.

An example. On Monday computer A pushes session X with 40 turns, and the
storage's history is v1, v2. A's record for X says "40 turns, accepted in v2".

- On Tuesday A continues X to 55 turns. The storage still holds 40 turns, and
  v2 is on the chain. The record is usable, only A changed, and the row reads
  "Update storage".
- Instead, on Tuesday computer B clears the storage and pushes again. The
  history is now w1. A's record still says "accepted in v2", and v2 is gone.
  The record is stale. Mallard ignores it and compares A's copy of X directly
  with the copy in w1.

Today a stale record blocks the user with "unlink and relink", "reset local
sync state", or "could not be proven safely". After this plan:

| Rule | |
|---|---|
| 0 | A stale record is ignored, as if it were not there. It is never deleted when a page refreshes |
| 1 | No usable record, and the storage does not have the session: **Push uploads it**, under the id it already has |
| 2 | No usable record, and the storage has an equal or an earlier version of the session: Mallard proves that from the content. An equal version is marked as synced. An earlier one is **updated by Push**, with the complete local session |
| 3 | No usable record, and the storage has a longer or a different copy: nothing is replaced. Push says "Pull required", and **Pull offers "Keep both"** |
| 4 | A Push that loses a race before it is confirmed **fails as a whole**. Nothing is half applied. One that is outranked after it was confirmed did not fail: the next refresh reports it `Displaced`, and nothing is rolled back. Either way the user compares and pushes again |

No case asks for a reset or an unlink. The first half of rule 4 is how Mallard
already works; the second is the decision of September 22. Both are written
here so that the case list is complete.

## 2. Every case

Row labels are the ones the app shows. "As today" means this plan does not
change the row. In the column "Usable record", "no" means that the record is
missing or stale.

### 2.1 One session

| # | This computer | The storage | Usable record | Push row | Pull row | Today |
|---|---|---|---|---|---|---|
| S1 | has the session | has no copy | no | "Publish new task", under the id it already has | "Keep local" | With a stale record: "Can't sync" on both. Without a record: as today |
| S2 | has it | has an equal copy | no | no change | "Mark synced" | With a stale record: compared against the stale record. Without: as today |
| S3 | has it | has an earlier version: the same session with fewer turns | no | "Update storage" | "Ready to push" | Same as S2 |
| S4 | has it | has a longer or a different copy | no | "Pull required" | "Keep both" | "Can't sync" on both. Reset does not help |
| S5 | does not have it | has it | no | "Not on this computer" | "Create local task" | As today |
| S6 | has it | has it | yes, nothing changed since | no change | no change | As today |
| S7 | has it, continued here | has it | yes | "Update storage" | "Ready to push" | As today |
| S8 | has it | has it, continued elsewhere | yes | "Pull required" | "Update this task" | As today |
| S9 | has it, continued here | has it, continued elsewhere | yes | "Pull required" | "Keep both" | As today |
| S10 | has it | has no copy | yes | "Can't sync" | "Can't sync" | As today. It cannot happen inside one history: the engine refuses a Push that drops a session |

After S4's "Keep both", this computer has the stored session and its own copy,
marked "Preserved". The preserved copy is then an S1 row.

**Reading a row, with S7 as the example.** The record says "40 turns, accepted
in v2". v2 is on the chain, so the record is usable. The stored copy still has
exactly those 40 turns, so no other computer changed it. The local copy has 55,
so this computer did. Because the stored copy is exactly what this computer
last agreed on, it holds nothing this computer has not seen, and the complete
local session can take its place: "Update storage". A Pull has nothing to
install for this session and only reports "Ready to push". After the Push the
record says "55 turns, accepted in v3", and the other computer sees S8.

A row decides one session. The storage area decides whether Push is open at
all (section 2.2): when another computer pushed anything into the same area
since, a Pull comes first, and S7 rows wait for it.

### 2.2 One storage area

A storage area is one agent's home, or the project's files. Push is open only
when every area it writes is A1, A3 or A5.

| # | The area in the storage | Base | State | Push | Today |
|---|---|---|---|---|---|
| A1 | absent | none | `Fresh` | open | As today |
| A2 | absent | usable | `RemoteMissing` | Pull required | As today. It cannot happen inside one history |
| A3 | absent | stale | `Fresh` | open | Push fails: "unlink and relink the storage" |
| A4 | present | none | `Unreviewed` | Pull required: "this storage already holds data for this project that has never been reviewed on this computer" | As today |
| A5 | present, same content as the base | usable or stale | `InSync` | open | As today |
| A6 | present, other content | usable | `Advanced` | Pull required | As today |
| A7 | present, other content | stale | `Unreviewed` | Pull required, as A4 | Push fails: "… pull and review …, or unlink and relink the storage" |

### 2.3 Project files

In a project that is not managed by Git, the project's files are one area, and
a file inside it has no record of its own: it is compared against the manifest
the area's base names. With no usable base, a file is compared by content
alone. The file labels are those of the files review; the Pull labels are
those of the Pull review.

| # | This computer | The storage | Files review | Pull row | Today, with a stale base |
|---|---|---|---|---|---|
| F1 | has the file | has no copy | "Only on this computer. Push to save it in storage." | none | "State unknown. Rescan to compare." The Push is held until the file is excluded: "Exclude the items that can't be captured". A rescan changes nothing, and in story 1 there is nothing to pull, so the files cannot be pushed at all |
| F2 | has it | has an equal copy | "Up to date here and in storage." | none | "State unknown. Rescan to compare." until a Pull saves a usable base |
| F3 | has it | has another copy | "Changed here and in storage. Review before syncing." | "Replace file · keeps local copy" | As F2. When the old manifest can still be read, the file is judged against it, and a file this computer changed there can read "Newer in storage" and be replaced plainly |
| F4 | does not have it | has it | "Only in storage. Pull to download it here." | "Add file" | As F2 |

With a usable base the files review works as it does today. Section 4.7 says
how the stale base is kept out.

### 2.4 What each computer sees

**Story 1. Computer B clears the storage. Nobody has pushed since.**

| Computer | Sees | Does |
|---|---|---|
| B | Its records were reset when it cleared, as today. Every session is S1 | Nothing yet |
| A | Notice: "Another computer cleared <storage>. Push again." Every area is A3, so Push is open. Every session is S1, and every project file is F1 | One Push recreates the storage. The notice goes away |
| B, afterwards | Its areas now hold A's data: A4. Its Pull review shows S2, S3, S4 and S5 rows, and "Keep local" for sessions only B has | One Pull, then Push uploads what the storage lacks |

**Story 2. Computer B clears the storage and pushes again.**

| Computer | Sees | Does |
|---|---|---|
| A | Notice: "Another computer cleared <storage>. Pull, then push again." Areas B wrote into are A7. Areas B did not write into are A3 | One Pull. Its review shows S2, S3, S4 and S5 rows, and F3 and F4 rows for the project's files. Then Push: S1 and F1 rows upload what the storage lacks, S3 rows update it |

When B wrote into none of A's areas, every area is A3 and A pushes without a
Pull, as in story 1.

**Story 3. Computer A clears the storage itself.** As today: A's records are
reset at once, every session is S1, and A pushes again.

**Story 4. The storage only looked empty**, for example a folder storage
inside a sync product that has not finished downloading.

| A does | Result |
|---|---|
| Nothing | When the listing is complete again, every record is usable again. Everything reads exactly as before |
| Pushes | The Push writes a second first version. When the real history shows up, the earlier first version is selected, and A reads `Displaced` as in R3: nothing is rolled back. A's areas are A7, or A5 where the content is equal: one Pull, with S2, S3 and S4 rows, then Push |

**Story 5. Reset local sync state on A.** As today, except that a session whose
stored copy is longer or different is S4 instead of "Can't sync".

**Story 6. A's confirmed Push was outranked, and the winner changed the same
session.** A's acceptance of that session is stale and read as absent (R3).
It is S2, S3 or S4. Today S4 is "Can't sync".

### 2.5 Races

Every race ends with one Push selected. The other fails as a whole when the
race is decided before it is confirmed (R1, R2, R5), and reads `Displaced` when
it is decided afterwards (R3); nothing is rolled back either way. Its user
compares and pushes again. Nothing is merged, and nothing is half applied.

| # | Race | What happens to A's Push | A afterwards |
|---|---|---|---|
| R1 | B pushed after A's review and before A's Push wrote its record | It fails before it writes: "The storage changed while pushing. Compare again." A's records are untouched | Compare again. B's Push makes A's areas A6: Pull, then Push |
| R2 | A and B write onto the same version in the same seconds | Both records are kept, and the one with the earlier stamp is selected. When that is B's, A's Push fails with the same message, and A's records are untouched. A's record stays in the storage and is never used | As R1 |
| R3 | A's Push was confirmed, and B's record with an earlier stamp shows up afterwards | At the next refresh the storage reads `Displaced`. Nothing is rolled back: A's bases and acceptances stay and are read as absent. Standing notice: "Another computer pushed to <storage> first. This computer's last push is no longer part of its history. Compare again." | Compare again. Areas B changed, and areas that hold A's displaced content, are A7: Pull, then Push. In that Pull review A's own files read "Replace file · keeps local copy"; left unticked they stay, the area is marked as reviewed, and the Push publishes them. A session B also changed is S2, S3 or S4 |
| R4 | A and B both push the first version onto an empty storage | Two first versions are kept, and the earlier one is selected. The other computer gets R2 or R3 | As R2 or R3 |
| R5 | B clears the storage while A is pushing | When A's parent version is already gone, A's record connects to nothing and is never selected: A's Push fails as in R2. That record stays in the storage and does not close it: a first version needs a storage that selects nothing, not an empty listing. When A's Push was confirmed before the clear finished, its records are stale | Story 1 or story 2 |
| R6 | A clears the storage while B is pushing | The mirror of R5, on B | Story 1 or story 2, on B |
| R7 | B pushes between A's Pull review and A's Pull | A's Pull is refused before it changes anything | Review again |
| R8 | B pushes while A's Pull is installing | A's Pull finishes against the version it reviewed; stored objects never change. B's version shows at the next refresh | A6 and S8 rows |
| R9 | The connection drops after A's record was sent | "Couldn't confirm the push. Refresh to see whether it arrived." | When it did not arrive: push again. When it arrived: L3 |
| R10 | Library: B pushes while A pushes | A's Push fails as a whole, and nothing is accepted | Compare again |

R1 to R3 were implemented and tested before this plan. R4 and R6 follow from
the same two checks, and this plan adds their tests. R5 needed one change: see
*Implementation notes*.

R2, R3 and R4 need two records under one version. On R2 and on Dropbox the
claim of the CAS-free plan (decision 11 and section 7.2; implemented on
September 22 as its step 9) lets a version have one child, so there they end as R1 while the
storage's claims are intact: the second Push fails before it writes. An
interrupted clear can remove a claim and reopen R3 there (CAS-free plan,
section 12). They remain as written on a local folder.

### 2.6 Library

| # | Case | After | Today |
|---|---|---|---|
| B1 | A repository was cleared, and nobody pushed since | The page loads and writes nothing: the link's accepted state is read as absent. Rows that exist only here are new and pushable. One Push repopulates the repository and saves a new accepted state | The page and its Push review fail: "the accepted resource repository disappeared from storage" |
| B2 | A repository was cleared and pushed again | The accepted state is read as absent; nothing is written at load. Rows on both sides compare as unreviewed and go through the Pull choice. Rows only here are pushable | As today |
| B3 | A changed storage identity, or a foreign repository | Read as absent, like B1 and B2: the accepted head describes a history this storage does not hold, so the page loads and writes nothing, every row compares as unreviewed, and the next Push or Pull saves a fresh accepted state. Changed on September 22, after this plan shipped: the refusal left the page unable to load, and with it Unlink disabled, the row's deletion and the storage's removal refused while the link existed. Only the identity can differ in practice; a head that names another repository never loads, because the links document refuses it | Refused: "the storage connection identity changed after this resource was accepted", or "the storage link points at a different resource repository" |

### 2.7 Cases this plan leaves as they are

| # | Case | What the user sees |
|---|---|---|
| L1 | A session's local file is gone | "Can't sync" |
| L2 | A session that cannot be read, or a record whose parts contradict each other | "Can't sync". It is not a history problem, and nothing can be compared |
| L3 | A Push whose record arrived while its local save never ran | A continued session reads "Keep both". A new session appears twice: once here, once in the storage |
| L4 | After a clear, a session the other computer had continued further than this one | S4, "Keep both", although a plain update would do. Nothing is lost; there is one extra "Preserved" copy |

## 3. Why a stored session is never replaced

The owner's rule is: when this computer cannot decide a session's status, the
session can be pushed instead of being blocked. "Cannot decide" covers two
situations:

| Situation | How often | What a Push does |
|---|---|---|
| The storage has no copy of the session (S1) | Every session, on every other computer, after a clear | It uploads the session. Nothing can be lost |
| The storage has a copy, and this computer has no usable record of it | Rare. It needs the same session on two computers | It would have to replace the stored copy |

Before it gives up on a session, Mallard checks whether the stored copy is
equal to the local one or an earlier part of it (S2, S3). So what is left, S4,
is a stored copy that holds turns this computer does not have. Replacing it has
two costs, read from the current code:

- Those turns leave the current version, and nobody is told.
- The other computers rely on a stored session only ever growing. When the
  replacement has more turns than they last synced, their Pull reads it as a
  plain update and installs it over their own copy. Claude sessions are
  replaced in place, so the turns are then gone on both computers.

The second cost is also why a stale record is never used, even for a session
the storage holds (rule 0). After a clear, the stored copy can come from the
other computer's own branch of the session. Compared against a stale
acceptance, it can look like a plain update.

The rule for a whole storage area does not change. When another computer has
pushed into it, a Pull comes first. A Push reads "stored, but not captured
here" as a deletion, so a Push over files this computer never reviewed would
propose deleting them on every other computer.

Two risks are accepted:

- **The storage only looked empty.** Story 4. Nothing is lost.
- **A clear forgets deletions.** A file that was deleted before the clear, and
  that some computer still has, is uploaded again. The Push review lists it.

### What an equal or an earlier version means (S2, S3)

A Push never uploads part of a session. It uploads the complete session as one
file, and the session's id then points at that file. The previous version stays
in the storage's older versions. An *earlier version* is therefore a complete,
older copy of the same session: the same conversation with fewer turns.

It can come from either computer. This computer pushed it before its record was
reset or displaced, or another computer that holds the same session pushed
its copy first, which is the usual case after a clear. The two copies are
paired by the session's cloud id, which a clear and a reset keep.

The proof needs no download. Every stored version carries its number of records
and a running digest over them. Mallard computes the same digest over the first
records of the local session. When they match, the stored version is exactly
the beginning of the local one, so the complete local session can take its
place without losing a turn. When the counts are equal as well, the two copies
are equal, and nothing is uploaded.

Pushing the local session as a new session instead would need no proof. It is
rejected: after a clear, nearly every session that both computers hold would be
stored twice, and after the next Pull each computer would have it twice.

## 4. How it is built

### 4.1 One rule for both records

A record counts only while the version it names is on the selected chain. For a
scope base that is the publication it was reviewed in. For a session acceptance
it is the publication it was accepted in. `Mainline` gains a lookup by commit
id for the acceptance, which stores no revision.

A scope base has one more way to count: the scope still has exactly the content
that was reviewed (A5).

The rule holds for every reader of a record, not only the classifiers.
Section 4.7 lists the two readers that open the manifest a base names.

### 4.2 Storage areas and the Push gate

`classify_scope_continuity` follows section 2.2. `ScopeContinuity::Replaced`
loses its only producer and is removed.

The Push gate stops returning early for a missing or a replaced history. The
existing rows decide: `Ready` when every writable scope is `Fresh` or `InSync`,
otherwise `RequiresPull`. An empty stream has no manifest, so every writable
scope is `Fresh`. That Push writes the project's first record with content.

A base that an outranked Push left behind reaches this table like any stale
base: it is read as absent (A3, A5, A7), and the comparison reports `Displaced`
so that the notice can say why (CAS-free plan, section 9.4). Nothing is rolled
back.

### 4.3 Sessions

A stale acceptance is treated as absent wherever the review reads it: where
the adoption probe is chosen, where the accepted evidence is assembled, and in
the pins. The acceptance of its subagents goes with it. The review does this in
one place: it reads every saved checkpoint through the selected chain when it
loads them, and only checkpoint maintenance, which writes the read cursor back,
works on the saved values. The selected chain therefore reaches the session
review at its call sites, and Pull apply reads its snapshot through the same
observation.

- **S1.** The evidence is "local, not in the storage, no acceptance", which the
  existing rules read as "Publish new task". The session keeps its mapping, so
  it is published under the cloud id it already has, and a second computer that
  holds the same session converges on that id.
- **S2 and S3.** First-contact adoption runs as it does today for a session
  without an acceptance.
- **S4.** When adoption cannot prove it, the pair classifies as `Divergent`
  instead of failing as `UnacceptedOverlap`. The existing matrix then gives
  "Pull required" and "Keep both". `TailEvidenceBlocker::UnacceptedOverlap` is
  removed. The same holds for a subagent pair.

L4 is the price of S4. Proving "the local copy is an earlier part of the stored
one" needs the stored transcript at review time. It is left out.

### 4.4 Nothing is deleted at refresh

An empty or a short listing is not proof of a clear. Sections 4.1 to 4.3 only
ignore a record, and they decide again at every comparison. Adoption at review
writes nothing either; its proof becomes a record only through "Mark synced" or
a Push.

The stale records go away when the user acts. A Push and a Pull save new bases
and new acceptance as they do today. In the same transaction, both remove every
scope base of the link that is off the selected chain, so that the notice of
section 5 ends.

### 4.5 Library

`verify_library_continuity` returns `Replaced` when an accepted head exists and
the repository lists empty, or when that head is not on the chain. The page then
reads the accepted state as absent, the head and every per-resource acceptance,
and writes nothing: the reset at load that B1 and B2 first specified is
withdrawn by the decision of September 22. A Library Push publishes only the
rows the user selected and carries the rest, so it cannot replace or delete
what it has not seen.

The Library's accepted state is one head and one acceptance per resource, and a
row's state reads only the acceptances. The reset used to empty them. Without
it, the next save has to, or old acceptances come back:

- **A Push or a Pull applied from an accepted state that reads as absent drops
  every saved acceptance, then saves the ones it accepted itself.** Otherwise a
  Push of one row saves a head on the chain, and the acceptances of every row it
  did not select count again, although they describe a history the storage no
  longer has. A row this computer changed on a displaced branch would then read
  as changed only in the storage, and its Pull would update it without keeping
  a local copy.
- **Such a Pull always saves the head it reviewed**, also when it did not accept
  every row. Otherwise the acceptances it just saved stay under the old head,
  are read as absent, and the rows it installed ask for review again. The rows
  it did not accept have no acceptance and still compare as unreviewed.

Apply decides "reads as absent" again from the observation that already checks
the reviewed head, not from the review. The link's revision check still refuses
a link that changed since the review.

### 4.6 One forget rule

Clear storage forgets a session's acceptance and keeps its read cursor and its
mapping, through one method on the checkpoint: it clears the accepted version,
its local twin and the accepted subagents, and advances the generation. It is
the only caller: the rollback of R3, which called it too, is withdrawn by the
decision of September 22. A displaced acceptance is kept and read as absent.

### 4.7 Every reader of a base asks whether it is usable

The scope classifier and the checkpoint reader apply section 4.1. Two more
readers open the manifest that the project area's base names, and neither asks
whether the base is usable:

- the files review, for the state of each project file;
- the Pull planner, for the base digests that decide whether a replaced file
  keeps a local copy.

After another computer's clear, that manifest is usually gone. The files review
then reads every file "State unknown. Rescan to compare." with the warning
"Reviewed project-content base is unavailable", and the Push review holds the
Push with "Exclude the items that can't be captured". No rescan changes it: the
manifest never comes back. In story 1 the storage is empty, so no Pull can
save a usable base either, and the files can only be excluded. This was found at review on September 22 with a test
of story 1 on a project with files. The story tests of steps 1 to 10 used
sessions only.

When the manifest can still be read, the files are judged against a history the
storage no longer has. That happens when the app's in-memory object cache still
holds it from before the clear, after an interrupted clear, which deletes
records before manifests, and for a displaced base. A file this computer
changed there can then read "Newer in storage", and the Pull replaces it
without keeping a local copy.

Both readers take the project area's base through the rule of section 4.1
first. A stale base is read as no base, whether or not its manifest can be read:

- the files review compares each file by content alone (section 2.3);
- the Pull planner gets no base digests, so every file it replaces keeps a
  local copy, as it already does when the manifest cannot be read.

The warning stays for a usable base whose manifest cannot be read. Both readers
need the selected chain. The files inspection that reads the storage without
observing it observes it instead, as Pull apply already does (section 4.3).
None of this depends on the claims of the CAS-free plan.

## 5. What the user sees

One standing notice replaces the two "Storage history changed" banners. It
shows while the comparison reports a missing, a replaced or a displaced
history, and it goes away when a Push or a Pull has replaced the stale bases.

| When | Copy |
|---|---|
| Push is open (stories 1 and 2 without shared areas) | "Another computer cleared <storage>. Push again." |
| A Pull comes first (story 2) | "Another computer cleared <storage>. Pull, then push again." |
| The Pull review | No banner. A Pull of a recreated storage is an ordinary first Pull |
| A confirmed Push was displaced (R3), or a Pull of a publication that was displaced afterwards | "Another computer pushed to <storage> first. This computer's last push is no longer part of its history. Compare again.", or "last pull" when this computer did not publish the displaced publication |

The notice sits in the notice zones of the project activity page and the
project page, in the warning tone. It has no action: Push and Pull are already
on the page. The notices of R1, R2 and R9 exist and do not change; R3's is the
standing note above, in place of the one-time shell notice and its event.

Three things decide which form shows:

- **Which history.** `RemoteMissing` while the stream selects nothing. Then
  `Displaced` when at least one stale base was reviewed in a displaced
  publication (CAS-free plan, section 9.1), and `Replaced` otherwise. A
  displaced publication is still listed and connects to the chain, so the
  storage was not cleared after it was written. A Push and a Pull save every
  writable base in one publication and remove the stale ones, so a link holds
  both kinds only after an interrupted clear.
- **Push or pull.** "last push" when the displaced publication with the
  greatest revision was published by this computer's copy of the project (the
  replica token in the record's `published_by_origin`), and "last pull"
  otherwise. It costs one read of an
  immutable record, only while the history reads `Displaced`. The base's
  `last_push_at` and `last_pull_at` are not used: a Pull that fails in part
  keeps its previous `last_pull_at`, so after it they can name the wrong one.
- **The shape.** `RemoteContinuity::Displaced` carries the two revisions, as
  `Replaced` does, and `saved_by`, `push` or `pull`. To tell a displaced base
  from a cleared one, the classifier also takes the listed records that the
  chain does not hold. This is the only IPC change of the plan.

## 6. What is removed

```text
PushDestinationStatus::RemoteHistoryMissing, PushDestinationStatus::RemoteHistoryReplaced
the two early returns that produce them, and their two "unlink and relink" error strings
ScopeContinuity::Replaced
TailEvidenceBlocker::UnacceptedOverlap
the "Unlink storage" verb on the three continuity banners, and the props that carry it there
the remote_missing / remote_replaced storage notices of the project page
the replaced / remote_missing exception in canAcceptPullMetadata
the Pull review's "Storage history changed" banner
the Library refusal "the accepted resource repository disappeared from storage"
the tests that pin the explicit unlink and the blocked overlap
```

Removed by steps 12 and 13, the behaviors the decision of September 22
withdraws:

```text
the rollback transaction in observe_linked_bundle, and what only it uses
the project-sync-push-outranked event, its forwarder, the shell notice and pushOutrankedNotice
the Library reset at load (reset_accepted_state)
the tests that pin the rollback and the reset; they are rewritten as Displaced and read-rule tests
```

Unchanged: every persisted document, the storage layout, the decision matrix
for Push and Pull, first-contact adoption, "Keep both", the two checks behind
R1 and R2, the detection behind R3 (`Mainline::outranked`), and what Clear
storage deletes. `RemoteContinuity` keeps its variants and gains one,
`Displaced` (section 5).

## 7. Work plan

| Step | Work | Done when |
|---|---|---|
| 1. Checks first | Three behaviors the plan rests on: a Push onto an empty history writes a first record with content; "Keep both" applies for a pair without an acceptance; adoption runs for a session whose acceptance is stale | Each has a failing or a passing test that says so |
| 2. Scope table | Section 2.2 in `classify_scope_continuity`; remove `ScopeContinuity::Replaced`; `Mainline` lookup by commit id | The domain suite passes with one fixture per row A1 to A7 |
| 3. Session pair rule | S4 in `VerifiedTailEvidence::new` and `compare_conversation_tail`; remove `UnacceptedOverlap`; the plan check accepts "Keep both" for a pair without an acceptance | A pair without an acceptance reads `Divergent`, for sessions and for subagents |
| 4. Forget rule | The method of section 4.6; Clear storage calls it | The persistence suite passes |
| 5. Push gate | Remove the two early returns, the two `PushDestinationStatus` variants and their strings; Push apply and Pull apply remove off-chain scope bases in the transaction that saves the new ones | A cleared, empty storage reads `Ready`. A recreated one reads `Ready` or `RequiresPull` by its scopes |
| 6. Stale acceptance | Pass the selected chain into the session review; treat a stale acceptance as absent in the evidence and in the adoption probe; Pull apply reads its snapshot through the same observation | The story tests of section 8 pass |
| 7. Library | `Replaced` for an empty repository with an accepted head; remove the refusal and its assertion | The catalog suites pass |
| 8. Frontend | The standing notice in its two forms; remove the Unlink verbs, the two storage notice kinds, the two Push states and the Pull review banner; the `canAcceptPullMetadata` change; the type mirrors. The notice goes into the static specimen first | Frontend suite and type check pass |
| 9. Docs | `DESIGN_REMOTE_HISTORY_CONTINUITY.md` becomes design history; sections 9.1, 9.2, 11, 12 and 16 of the CAS-free plan; `CURRENT_METADATA_LAYOUT.md`, `TECHNICAL_REPORT.md`, `README.md` | The documents describe this behavior |
| 10. Gate | `cargo test --workspace --all-targets`, `npx tsc --noEmit`, the frontend integration suite, `npm run build` | All pass |

Steps 1 to 14 are implemented and committed; each left the workspace building
and its own tests green. They needed nothing from the claims of the CAS-free
plan (its step 9) and landed before them. They take over the parts of the CAS-free plan's step 10
that are not about publishing: the rollback, `Displaced`, the base readers and
the Library read rule. The rest of that step (the request limits,
`OutcomeUnknown`, `PushUnfinished` and *Finish that push*) stays there.

| Step | Work | Done when |
|---|---|---|
| 11. Base readers | Section 4.7: the files review and the Pull planner read the project area's base through the rule of section 4.1; the files inspection observes the storage instead of reading it | Rows F1 to F4 pass after a clear and after a displacement; with the old manifest still readable, no file is judged against it; no file reads "unknown" after a clear |
| 12. `Displaced` | `RemoteContinuity::Displaced` with `saved_by`, and its TypeScript mirror; the classifier takes the unselected records; the "last push" or "last pull" read of section 5; remove what section 6 lists for this step; the notice's third form, in the static specimen first | R3 and story 4 pass as rewritten: the next refresh reads `Displaced` and the link's saved state is byte-identical before and after it; the frontend suite asserts every text of section 5 |
| 13. Library | Section 4.5: the read rule at load, and the save rule in Push apply and Pull apply; remove the reset at load | B1, B2 and the Library rows of section 8 pass; the page writes nothing at load |
| 14. Gate | As step 10, and the documents: the CAS-free plan's step 10, and its sections 9.4, 11 and 13, point here for these parts; `CURRENT_METADATA_LAYOUT.md` and `TECHNICAL_REPORT.md` where they describe the rollback or the reset | All pass |

## 8. Tests

Every changed row of section 2 gets a test that names it.

| Cases | Expected |
|---|---|
| A1 to A7 | One domain fixture per row |
| S1, story 1 | Push is `Ready`. Every session reads "Publish new task". One Push writes the first record, under the cloud ids the sessions already had. The next refresh reads in sync. No unlink, no reset, no Pull |
| Story 2, other areas only | Push is `Ready` without a Pull |
| Story 2, same area | Push reads `RequiresPull`. After the Pull, Push publishes the S1 rows, and no row reads "Can't sync" |
| S2, S3 with a stale record | "Mark synced" and "Update storage", by adoption |
| S4 | Push reads "Pull required". Pull reads "Keep both" and applies: the stored session is installed, and the local one is kept as "Preserved" |
| Rule 0, the other computer's branch | A stale acceptance, and a stored copy from another branch with more turns: the row is S4, never "Update this task" |
| Story 4, nothing done | With no Push or Pull in between, the link reads exactly as before the listing was short |
| Story 4, pushed | A reads `Displaced` as in R3, nothing is rolled back, and the areas read A7 or A5 |
| Stale bases | After the Push or the Pull, the link holds no scope base off the selected chain, and the notice is gone |
| Forget rule | Clear storage leaves the checkpoint with its cursor and without an acceptance |
| R4 | Two first versions: the later one fails, or reads `Displaced` |
| R5 | A record whose parent was cleared is never selected, and that Push fails as a whole |
| B1 | The page loads and writes nothing. A local row is selectable. One Push repopulates the repository |
| B3 | Read as absent since September 22, like B1 and B2; the page loads |
| F1 to F4, story 1 with project files | A project with files, cleared by another computer: every file reads "Only on this computer", no file reads "unknown", Push is `Ready`, and one Push publishes the files |
| F3 and F4, story 2 with project files | After the other computer pushed files again: a file both hold with other content reads "Changed here and in storage", and the Pull offers "Replace file · keeps local copy"; a file only the storage has reads "Add file" |
| A readable stale manifest | After an interrupted clear, and for a displaced base, the old manifest can be read: no file is judged against it, and no replaced file loses its local copy |
| The notice's third form | "last push" after a displaced Push; "last pull" after a Pull of a publication displaced afterwards, also when that Pull failed in part; `Displaced` wins over `Replaced` on one link |
| Library save rule | From an accepted state read as absent (B2, and a displaced head): a Push of one row leaves no acceptance for the rows it did not select, and they compare as unreviewed after it; a Pull that accepts some rows saves the reviewed head, and the rows it installed read in sync |
| Rewritten | `missing_remote_history_requires_an_explicit_unlink_before_republishing`, `replaced_remote_history_blocks_push_on_every_surface_and_pull_heals_it`, `push_destination_classification_preserves_remote_history_safety`, the domain rows that expect `UnacceptedOverlap`, and the frontend tests that assert the Unlink verb or the two Push states |
| Rewritten by steps 12 and 13 | The three rollback tests become `Displaced` tests, and the Library tests that expect a reset at load expect the read rule |

## 9. Not in this plan

- **Replacing a stored session, or pushing over an area this computer has not
  reviewed.** Section 3 gives the reason.
- **L1 and L2.** They are not history problems.
- **L3.** Its fix is to recreate the session mapping from a manifest this
  computer published. It is its own plan.
- **L4.** It needs the stored transcript at review time.
- **A marker that records a clear.** It would make a clear provable, and it is
  a storage layout change.

## Implementation notes

Implemented on September 20, 2026. What differs from the plan above:

| Subject | What was built |
|---|---|
| R5 and an interrupted clear | The plan said R4 to R6 follow from the two existing checks. R5 did not. List-then-write admitted a first version only into a storage whose listing was empty. A record that landed after its parent was cleared, and every record an interrupted clear left behind, connect to no chain, so the listing was not empty and the storage could never take a first version again. Every later Push failed with "The storage changed while pushing. Compare again." A first version is now admitted when the storage selects nothing. The exposure this adds is the tail of a clear's record pass, which a first version written during a clear already had for the whole payload pass (CAS-free plan, section 12). The test of the interrupted clear was rewritten, and sections 7.2 and 12 of the CAS-free plan say so |
| Withdrawn on September 22 | The rollback of R3 (`observe_linked_bundle`) and the Library reset at load were built as planned and are withdrawn by the decision that recovery is a user action: a displaced base or acceptance is kept and read as absent, the comparison reports `Displaced`, the standing notice says so, and the user compares and pushes again. The CAS-free plan, section 9.4 and step 10, carries the change |
| One read rule for a checkpoint | Section 4.3 named two read sites. The review has more: the pins, the "durably pinned" flag and the subagent pins also read the acceptance. `ConversationCheckpoint::read_on` reads a whole checkpoint through the selected chain, the review maps every saved checkpoint through it once, and checkpoint maintenance is the only code that keeps the saved values. Nothing is written by reading |
| The standing notice | It sits in the notice zone of the project activity list, which the project page embeds and the Push review shows above its list. It is derived from the comparison the page already has: the history reads `remote_missing` or `replaced`, and Push is open when every area this computer writes reads `fresh` or `in_sync`. No IPC payload changed. Step 12 adds the `displaced` form and the one IPC change of section 5. The storage card keeps its event notices; the two kinds that the removed Push states produced are gone |
| `canAcceptPullMetadata` | It refuses only a storage that could not be listed. A recreated storage whose rows all read "Mark synced" is accepted without opening the review, as any first Pull is |
| Library B1 | Built as first planned: the page resets the link's accepted state when the repository lists empty, as it already did for B2. That made a short listing (story 4) change saved state. Withdrawn on September 22 with the reset for B2; step 13 replaces both with the read rule and the save rule of section 4.5 |
| Project files, found at review on September 22 | Steps 1 to 10 did not cover the files review or the Pull planner, which open the project area's base manifest without the rule of section 4.1. A test of story 1 on a project with files, run and then removed, read both files "unknown" with the warning "Reviewed project-content base is unavailable", while the project area read `Fresh` and the history `RemoteMissing`. The story tests used sessions only. Step 11 fixed it on September 22 (section 4.7): `project_base` is the one rule both readers take the base through, and row A5 reads the selected manifest as the base, because it holds exactly the reviewed content and the base's own manifest may be gone; the files inspection observes the stream. Story 1 with files, story 2 with files and a readable stale manifest after an interrupted clear are tested in the service suite. The readable manifest of a displaced base is tested with step 12: until the rollback goes, it moves that base before the readers see it |
| Tests that were rewritten | `push_destination_is_decided_by_the_writable_scopes_alone`, `a_cleared_storage_is_pushed_again_without_an_unlink_or_a_reset`, `a_recreated_storage_asks_for_a_pull_and_the_pull_replaces_the_stale_bases`, `a_recreated_history_reads_as_unreviewed_and_a_live_one_as_advanced`, and the two tests that pinned the blocked overlap, which now expect "Pull required" and "Keep both" |
| Step 12, `Displaced` | Built on September 22. `RemoteContinuity::Displaced { base_revision, remote_revision, saved_by }`, with `SavedBy::Push` or `Pull`. The classifier takes the listed records the chain does not hold, and asks once, with the newest displaced publication, who published it: `BundleEngine::published_by` reads that record, and its replica token is compared with this binding's. `Mainline::outranked` names the displaced record; `Outranked.shared`, the rollback transaction, its event, its forwarder, the shell notice and `pushOutrankedNotice` are gone. `observe_linked_bundle` returns the observation with the verdict, and takes the link by reference. The three rollback tests are `Displaced` tests that compare the saved state byte for byte before and after the refresh; the readable manifest of a displaced base is tested with the files world of step 11. The notice's third form is in `clearedStorageNotice.ts` and in the static specimen. Story 4's test gives the second first version one more session than the real history, so its area reads A7 by construction: with identical sessions the conversation entries in the area digest still carry their `updated_at`, and A5 or A7 then depends on whether the two captures fell in the same second, which the story allows |
| Step 13, Library | Built on September 22. `resolve_scope_workspace` reads the accepted state as absent when `verify_library_continuity` says `Replaced`, on the workspace's copy of the link only (`stale_acceptance`), and writes nothing; `reset_accepted_state` is gone. When the state was stale, the Push save clears the saved acceptances before it saves its own, and the Pull save does the same and then always saves the head it reviewed. Tests: B1 rewritten; B2 with the Push half of the save rule; a displaced head, a rival record planted on the first publication after two pushes, with a Push; B2 with the Pull half. The rival planter takes the chain's root as the parent, so it works with more than one record |

Still open: the live two-computer pass on R2 and on Dropbox. The owner runs it,
after clearing `~/.mallard` and the cloud data.

## Source pointers

Line numbers are those of the working tree on September 20, 2026, after steps
1 to 10, except the rows that steps 11 to 13 touched, which were read after
them on September 22.

| Subject | Location |
|---|---|
| `ScopeBase`; `ScopeContinuity`; `classify_scope_continuity`; `RemoteContinuity`; `SavedBy`; `classify_remote_continuity` | `crates/mallard-domain/src/lib.rs:872`, `:899`, `:931`, `:964`, `:1009`, `:1024` |
| Continuity decision tables, with the `Displaced` rows; rows A1 to A7 | `crates/mallard-domain/src/tests.rs:1252`, `:1424` |
| `Mainline`: selection, `contains`, `contains_commit`, `outranked`, which names the displaced record | `crates/mallard-domain/src/publication.rs:219-331` |
| `AcceptedConversationVersion`, with the commit it was accepted in; `ConversationCheckpoint`; `forget_acceptance`; `read_on` | `crates/mallard-domain/src/conversation_tail.rs:341`, `:540`, `:570`, `:589` |
| Tests of the read rule and the forget rule | `crates/mallard-domain/src/tests.rs:2511`, `:2563` |
| The evidence shapes that block; `compare_conversation_tail`, with the pair rule of S4 | `crates/mallard-domain/src/conversation_tail_relation.rs:166`, `:222` |
| Push and Pull decision matrix | `crates/mallard-domain/src/conversation_reconciliation.rs:168-222` |
| The plan check that ties an action to its relation | `crates/mallard-domain/src/lib.rs:3510` |
| S4 as a relation, a decision and a plan action | `src-tauri/src/sync_tests/thread_variants/relation_matrix.rs:206` |
| The texts of R1, R2 and R9; `admits`; the two checks in `write_record_at` | `crates/mallard-infrastructure/src/storage_core/publication.rs:46-48`, `:151`, `:425` |
| Tests of R4 and R5 | `crates/mallard-infrastructure/src/storage_core/publication.rs:672`, `:710` |
| An interrupted clear still takes a first version | `crates/mallard-infrastructure/src/storage_core/mod.rs:813` |
| Push reads "stored, not captured" as a deletion | `crates/mallard-infrastructure/src/bundle_engine.rs:5850`, `:5938-5960` |
| Pull turns a file removal record into a delete proposal | `crates/mallard-infrastructure/src/bundle_engine.rs:1636-1687` |
| A session cannot be dropped without a removal record | `crates/mallard-infrastructure/src/bundle_engine.rs:6003` |
| R7: a Pull is refused when the storage moved since its review | `crates/mallard-infrastructure/src/bundle_engine.rs:5725` |
| What Clear storage resets locally | `crates/mallard-infrastructure/src/persistence.rs:1289-1340` |
| `observe_linked_bundle`, the `Displaced` classifier, which writes nothing; `BundleEngine::published_by` | `src-tauri/src/project_sync/service.rs:7122`; `crates/mallard-infrastructure/src/bundle_engine.rs:766` |
| `PushDestinationStatus`; `classify_push_destination`; `push_destination_error` | `src-tauri/src/project_sync/service.rs:7706`, `:7749`, `:7819` |
| `thread_review`; its production call sites | `src-tauri/src/project_sync/service.rs:3471`; `:7349`, `:7368`, `:10461`, `:10480`, `:10807`, `:10961` |
| Checkpoints read through the selected chain; the adoption probe; accepted evidence assembly | `src-tauri/src/project_sync/service.rs:5151`, `:5334`, `:5752` |
| First-contact adoption, Codex and Claude | `src-tauri/src/project_sync/service.rs:3882`, `:4355` |
| Pull apply reads its snapshot with the chain | `src-tauri/src/project_sync/service.rs:3271` |
| `drop_stale_scope_bases`; its calls in Push apply and Pull apply | `src-tauri/src/project_sync/service.rs:13730`; `:10644`, `:13140` |
| The Push gate, rows A1 to A7 | `src-tauri/src/project_sync/service.rs:16534` |
| Story 1 and S1; story 2 on one area; A6 against A7 | `src-tauri/src/project_sync/service.rs:20152`, `:21341`, `:22188` |
| Rule 0 with S4 and "Keep both"; S2 and S3 under a stale record | `src-tauri/src/project_sync/service.rs:21232`, `:21358` |
| Story 4, nothing done and pushed | `src-tauri/src/project_sync/service.rs:21972`, `:22051` |
| The `Displaced` tests: a confirmed Push, a first Push, story 4, and a readable manifest of a displaced base; an interrupted clear reads `Replaced` | `src-tauri/src/project_sync/service.rs:20951`, `:21180`, `:22051`, `:20633`; `:21282` |
| `verify_library_continuity`; the text of R10 | `src-tauri/src/storage_catalog/resource_engine.rs:86`, `:35` |
| The Library read rule at load (`stale_acceptance`); B1; B2 with the Push half of the save rule; a displaced head; the Pull half of the save rule | `src-tauri/src/storage_catalog/service.rs:1068`; `:4074`, `:3747`, `:3859`, `:3957` |
| Library row states without acceptance | `src-tauri/src/storage_catalog/resource_planning.rs:236-282` |
| The row labels of section 2.1 | `src/components/project-sync/ProjectActivityPage.tsx:1102-1140` |
| The standing notice and where it is shown | `src/components/project-sync/clearedStorageNotice.ts`, `src/components/project-sync/ProjectActivityPage.tsx:1840` |
| The static specimen of the notices | `mockups/push-notices-live.tsx`, `mockups/push-notices-live.html` |
| `canAcceptPullMetadata` | `src/components/project-sync/pullReviewFlow.ts:52` |
| Type mirrors: `ScopeContinuity`, `PushDestinationStatus`, `RemoteContinuity` with `displaced` | `src/types.ts:275`, `:1066`, `:1089` |
| Frontend tests of the notice, the Pull review and the Push states | `tests/frontend/project-activity.integration.test.tsx`, `tests/frontend/pull-review.integration.test.tsx`, `tests/frontend/resource-inventory.integration.test.tsx` |
| The rule of section 4.7, `project_base`; its two readers, the files review and the Pull planner | `src-tauri/src/project_sync/service.rs:2721`; `:2337`, `:2758` |
| The files inspection, which observes the stream | `src-tauri/src/project_sync/service.rs:2274` |
| Tests of story 1 and story 2 with project files, and of a readable stale manifest after an interrupted clear (`FilesWorld`) | `src-tauri/src/project_sync/service.rs:20415`, `:20582`, `:20638`, `:20700` |
| The planner keeps a local copy when a file's base digest is unknown | `crates/mallard-infrastructure/src/bundle_engine.rs:5040` |
| The in-memory object cache that can still serve a cleared manifest | `crates/mallard-infrastructure/src/bundle_engine.rs:180`, `:1790` |
| The Push hold on an "unknown" file | `src/components/project-sync/PushResourceWorkspace.tsx:99`, `:416` |
| Library: a row's state reads the acceptances alone; the Push save; the Pull save and its head | `src-tauri/src/storage_catalog/resource_planning.rs:87`; `src-tauri/src/storage_catalog/service.rs:1885`; `:2148` |
| The origin a Push records | `src-tauri/src/project_sync/service.rs:9435` |
| A Pull that fails in part keeps its previous `last_pull_at` | `src-tauri/src/project_sync/service.rs:13447` |
