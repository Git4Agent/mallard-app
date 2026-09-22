# Mallard: a Pull applies the version it reviewed — implementation plan

- Status: Planned, not implemented
- Last reviewed: September 22, 2026
- Scope: the desktop app — how a Pull behaves when the storage changes after its review or while it applies, what it keeps when it cannot finish, and what it asks of the user
- Compatibility: clean cutover; no migration, no legacy detection, no legacy execution path
- Deployment assumption: there are no customers yet; the owner clears `~/.mallard` and the cloud data before retesting
- Schemas: every schema and layout number stays `1`. No persisted document changes shape
- Related: `PLAN_CLEARED_STORAGE_PUSH_AGAIN.md` (the Push side; its rows S1 to S10, A1 to A7, its stories and R3 are used here by their ids), `DESIGN_CAS_FREE_STORAGE_REFACTOR.md` (sections 5, 6, 7.2, 9.3 and 12; the claim of its decision 11 and section 7.2, planned and not implemented, is what makes T3, Q3, Q4 and G2 impossible on R2 and on Dropbox)

Code locations are collected in *Source pointers* at the end, not in the body.

## 1. The rules

A Pull has two steps. The **review** compares this computer with one version
of the storage and lists what a Pull would change. The **apply** makes the
changes the user ticked.

A stored version never changes. Its record, its manifest and its objects are
written once, their names carry their digests, and every read is checked
against them. A Push adds a version on top; it never edits one. Only **Clear
storage data** deletes anything.

So the version a review looked at can be read again later, byte for byte, for as
long as nobody clears the storage. The review **pins** it: it remembers which
version it compared against. The pin is already there today. What changes is
what the apply does with it.

An example. Computer A opens a Pull review at 10:00. The storage's history is
v1 to v5, so the review pins v5. At 10:05 computer B pushes v6. At 10:06 A
presses Pull.

- Today the Pull is refused: "bundle head changed after comparison; compare
  again". A reviews again against v6, and can lose that race again.
- After this plan the Pull applies v5, exactly what A reviewed. v5 is still in
  the storage, and v6 was built on it. Afterwards A reads "changes to pull" for
  whatever v6 added, as after any Push by B.

| Rule | |
|---|---|
| 0 | A Pull applies exactly the version it reviewed, and reads only from that version |
| 1 | A newer version that was pushed after the review does not stop the Pull. It shows afterwards as changes to pull |
| 2 | The pinned version must still be part of the storage's history when the apply starts. When it is not, because the storage was cleared or the version lost a race, the Pull is refused before it changes anything: "Compare again" |
| 3 | When the storage can no longer be read while a Pull applies, because it was cleared, a synced folder has not finished replicating it, or the connection dropped, **the Pull stops**. What was installed stays installed and keeps its record. Nothing is rolled back, nothing is retried, and the storage areas are not marked as reviewed |
| 4 | An item that fails, fails alone. The other items are applied |
| 5 | No Pull case offers a reset or an unlink. An item that cannot be pulled can be left unticked, and the Pull goes on without it |

Rule 4 is how Mallard already works. It is written here so that the case list
is complete.

## 2. Every case

Row labels are the ones the app shows. "As today" means this plan does not
change the row. The texts in the "Today" columns are the backend's. The app
prints an apply error after "Restore failed:", and says "repo" where the
backend says "bundle".

### 2.1 Opening the review

| # | The storage | This computer's bases | The review | Today |
|---|---|---|---|---|
| O1 | holds nothing for this project, and this computer never synced with it | none | It does not open. When the storage holds other projects, the app offers to connect to one of them. Otherwise: "Pull couldn't start", with Retry | As today |
| O2 | holds nothing for this project, and this computer synced with it before: it was cleared | stale | It does not open. Notice: "There is nothing to pull from <storage>." The standing notice of the Push plan already says "Push again" | As O1: the chooser of other projects, or "Pull couldn't start" with "bundle '<id>' does not exist" |
| O3 | the version this computer last synced | usable | Nothing to review. The Pull accepts the storage's version without opening a review, and saves the "Mark synced" rows on the way: "Storage reviewed for <project> — files unchanged." | As today |
| O4 | a newer version on the same history | usable | It opens with the rows that changed | As today |
| O5 | data this computer never reviewed | none | An ordinary first Pull | As today |
| O6 | a history that was cleared and pushed again | stale | An ordinary first Pull, with no note about the clear (Push plan, section 5) | As today, since the Push plan |
| O7 | cannot be reached, or its credential was refused | any | It does not open. "Pull couldn't start", with Retry, or Reconnect when the credential is the cause | As today |
| O8 | a history too long to list within the budget | any | It does not open; the listing is reported as incomplete | As today |

The review pins the version it compared against, and it is valid for one day.

### 2.2 One session in the review

The rows are those of the Push plan, read from the Pull side.

| # | Pull row | What the apply does | Push plan rows |
|---|---|---|---|
| P1 | "Create local task" | Installs the stored session as a new local session | S5 |
| P2 | "Update this task" | Installs the longer stored session. Claude and the other single-file agents are replaced in place, with a backup of the replaced file. Codex gets a new rollout beside the old one | S8 |
| P3 | "Keep both" | Installs the stored session and keeps the local one as a "Preserved" copy | S4, S9 |
| P4 | "Mark synced" | Saves the record. No file is read or written. See K7 for when it is applied | S2 |
| P5 | "Ready to push", "Keep local", no change | Nothing. The row is information only | S1, S3, S6, S7 |
| P6 | "Can't sync" | Nothing. The row cannot be ticked | L1, L2, S10, and K1 to K3 of section 2.9 |

A row the user leaves unticked is not applied. An unticked "Update this task"
reads "Keep local".

### 2.3 One file in the review

| # | File | Pull row | What the apply does |
|---|---|---|---|
| F1 | an agent's own file, stored and different here | "Add file", "Replace file" or "Merge file" | Writes it, with a backup of the replaced file |
| F2 | a project file, stored and absent here | "Add file", with "Create folder" rows for its folders | Writes it, and creates its folders first |
| F3 | a project file changed on both sides, or changed here without a usable base | "Replace file · keeps local copy" | Copies the local file to a `-local-copy` sibling, then writes the stored one. The file side of "Keep both" |
| F4 | a project file that only the storage changed | "Replace file" | Writes it, with a backup |
| F5 | a project file the storage deleted | It is not shown in the Pull review today (K8) | Nothing |
| F6 | a file with no safe target in this build, or a blocked target | It is not shown in the Pull review today (K8) | Nothing |
| F7 | a Git-managed project folder | Project files are not offered | Nothing. Git carries them |

### 2.4 Between the review and the apply

| # | What changed | The apply | Today |
|---|---|---|---|
| T1 | nothing | Applies | As today |
| T2 | another computer pushed a newer version on top of the pinned one | **Applies the pinned version** (rule 1). Afterwards the areas that computer changed read A6, and the sessions it continued read S8: one more Pull brings them | Refused: "bundle head changed after comparison; compare again" |
| T3 | the pinned version lost a race: a record with an earlier stamp showed up beside it | Refused before anything changes: "The storage changed since this review. Compare again." | Refused, with the text of T2 |
| T4 | the storage was cleared, and is empty | Refused, as T3. After comparing again: story 1 of the Push plan | Refused: "bundle '<id>' does not exist" |
| T5 | the storage was cleared and pushed again | Refused, as T3. After comparing again: story 2 of the Push plan | Refused, with the text of T2 |
| T6 | more than a day passed | Refused: "This review is older than a day. Compare again." | Refused: "Pull review has expired or is not active yet" |
| T7 | a ticked session was continued or moved here | That session fails alone: "This conversation changed after the comparison. Compare again." The others are applied | As today. Some of these failures print an internal name instead, such as "is stale at Tail(LocalTail)" |
| T8 | a ticked file was changed here | That file fails alone: its target changed after the comparison. When the file now needs no change at all, the Pull is refused before anything changes: "resource '<id>' changed after comparison" | As today |
| T9 | the project folder, an agent profile or this computer's identity changed | Refused before anything changes | As today |
| T10 | the storage cannot be reached | Refused before anything changes | As today |
| T11 | this computer itself pushed or pulled a newer version since the review, from another window | Refused, as T3. The pin is older than what this computer already synced, and applying it would move its bases backwards | Refused, with the text of T2 |

Today a refused Pull leaves its review open under a red banner that has no
button. After this plan the banner of T3 to T6 and T11 carries **Compare
again**, which builds a new review, as "Rescan" on the files step already does.

**How a published version can still lose (T3).** On R2 and on Dropbox it
cannot. A Push there claims the version it builds on before it writes its
record: one create-only write under that version's name, which the storage
refuses for the second computer in the same request (CAS-free plan, decision
11 and section 7.2). A listed version has no sibling, so a review that pinned
it can never be refused for T3, and Q4 does not happen. The rest of this
paragraph is about a local folder, whose store writes no claim: there a
published version is final except in the first seconds after its record was
written. Mallard has no lock on the folder, so two computers can both see "v4
has no child yet" and both write a v5 record. Both records are kept, and every
computer selects the one with the earlier timestamp, not the one that arrived
first. The timestamp comes from the writer's own clock when the record is
built, before it is sent, so a slower upload or a clock that runs behind makes
the two orders differ.

| Time | Event |
|---|---|
| 10:00:00.1 | Computer C builds its v5 record. Its upload is slow |
| 10:00:00.9 | Computer B builds its v5 record. Its upload is fast |
| 10:00:01 | B's record lands. It is the only child of v4, so it is selected |
| 10:00:02 | Computer A opens a Pull review. It pins B's v5 |
| 10:00:03 | C's record lands. It has the earlier timestamp, so every computer now selects C's v5 |

B's v5 is still in the storage, and it is no longer part of the history anyone
selects. B's Push fails, or B reads `Displaced` at its next refresh (R2, R3),
and B compares and pushes again on top of C's version. A's pin names the
version that lost, so the apply refuses it.

A rival record can only come from a computer that listed the folder before the
pinned version was visible. Usually it sends within seconds of that listing,
so a version older than a minute rarely loses. But a computer that paused
between its listing and its send, or a replica of a synced folder that was
offline, can deliver a record with the older stamp days later, and nothing in
a folder can refuse it (CAS-free plan, section 7.2). So on a local folder no
version is ever certain to keep its place, the review cannot tell a safe
version from an unsafe one, and waiting would not help. The apply checks
instead: it lists the history anyway. A rival that shows up only after the
Pull finished is Q4, whenever it shows up.

### 2.5 While the Pull applies

The apply runs in this order. Steps 1, 2, 4 and 6 read the storage. Nothing
local has changed before step 3.

| Step | Work | Reads the storage |
|---|---|---|
| 1 | List the history, find the pinned version on it, read its record and its manifest | yes |
| 2 | Compare the ticked sessions once more with their local files. For a ticked "Create local task", read the stored session once, to refuse a stored subagent rollout (K3) | only for "Create local task" rows |
| 3 | Save the "Mark synced" rows that were sent along, in one local save | no |
| 4 | Download the ticked sessions, with their subagents and their side files, as one batch. Every object is checked against the manifest | yes |
| 5 | Install the sessions one by one, from the downloaded copies. Each one is its own local save, with its own receipt | no |
| 6 | Download and write the ticked files | yes |
| 7 | Save the result. Mark the storage areas as reviewed only when nothing that had to be applied was left out | no |

| # | Event | When | What happens | Today |
|---|---|---|---|---|
| D0 | The storage is cleared, or cannot be reached | during step 1 or 2 | The Pull is refused, as T3 or T10. Nothing local has changed | As today, with the engine's own text |
| D1 | Another computer pushes a newer version | after step 1 | Nothing. The Pull finishes against the pinned version (rule 1) | The sessions are installed. Then step 6 reads the head a second time, and the Pull fails: "bundle head changed after restore planning". The installed sessions stay, the error does not say so, and the areas are not marked as reviewed |
| D2 | The storage is cleared | during step 4 | **The Pull stops** (rule 3). No session and no file is installed | The Pull fails: "bundle object '<key>' is missing". No session and no file is installed |
| D3 | The storage is cleared | after step 4, before step 6 finishes | The downloaded sessions are installed, because their verified copies are already here. Then the Pull stops at the first file object it cannot read. The files that were not written yet are not written | The sessions are installed. Then the Pull fails: "bundle head disappeared after planning". The error does not list the installed sessions |
| D4 | The connection drops, or the storage refuses the credential | during step 4 or 6 | The Pull stops, as D2 and D3, with the storage's own error | The Pull fails with that error. After step 5 it does not list the installed sessions |
| D5 | One session cannot be installed: a local write fails, or a local file changed in that moment | step 5 | That session fails alone (rule 4). The others are installed | As today |
| D6 | A stored session does not match what its manifest says about it | step 5 | That session fails alone: "This stored session cannot be read by this version of Mallard." It can be left unticked. **No reset is offered** | That session fails alone: "cloud conversation changed lineage or semantic content". The result's only action becomes "Reset local sync state". A reset cannot help: the mismatch is in the stored data, and the same Pull fails again afterwards |
| D7 | Another Mallard operation saves sync settings on this computer in that moment | step 5 | The session being installed fails alone: "Sync settings changed while pulling. Pull again." | As today, with the text "sync config changed during Pull apply" |
| D8 | The app is quit, or the computer sleeps or crashes | any | Every session that was saved is complete and stays. A session whose files were being written may be left partly written; its replaced files are in the backup folder. Nothing resumes | As today |

D2 and D3 look the same when the storage is a folder inside a sync product
that has not finished replicating the version: the object is not gone, it has
not arrived. Nothing else makes an object of a listed version unreadable, and
Mallard cannot tell the two folders apart, so the Pull stops the same way and
says what it knows. Once the files have arrived, the next comparison shows the
version whole (N7).

A stopped Pull reports what it did. Its title is "Pull stopped", its first line
is "Part of this version is missing from <storage>. Compare again." or the
storage's own error, its counts read "3 restored · 4 not pulled", and every row
it did not reach reads "Not pulled". Nothing is rolled back, and nothing runs
again by itself. The user compares again.

### 2.6 After a Pull that stopped or left items out

| # | The Pull | What the next comparison shows |
|---|---|---|
| N1 | finished against the pinned version while a newer one existed (T2, D1) | The areas the newer version changed read A6, its continued sessions read S8, its new sessions read S5. Areas it did not touch read A5, and Push is open |
| N2 | stopped because the storage was cleared, and the storage is still empty (D2, D3) | Story 1 of the Push plan. The sessions this Pull installed hold a record of the cleared history, so the record is stale and they read S1: "Publish new task", under the cloud id they already have. One Push recreates the storage |
| N3 | stopped because the storage was cleared, and another computer has pushed again | Story 2 of the Push plan. The installed sessions read S1, S2, S3 or S4 |
| N4 | stopped because the connection dropped (D4) | The installed sessions read in sync. The others are offered again. The areas still read "Pull required" |
| N7 | stopped because a synced folder had not finished replicating the version (D2, D3) | Until the files have arrived, the comparison cannot read the version and the storage card says so. Then as N4 |
| N5 | finished, and some ticked items failed (D5 to D7) | The applied items read in sync. The failed ones are offered again. The areas are not marked as reviewed, so Push still says "Pull required". Leaving an item that cannot be pulled unticked lets the next Pull mark the areas as reviewed (rule 5) |
| N6 | finished with nothing ticked | The areas are marked as reviewed, and Push is open. "Mark as reviewed", as today |

### 2.7 Races

| # | Race | What happens to A's Pull | A afterwards |
|---|---|---|---|
| Q1 | B pushes between A's review and A's Pull | T2: it applies the pinned version | N1 |
| Q2 | B pushes while A's Pull applies | D1: nothing | N1 |
| Q3 | The pinned version loses its race before A's Pull starts | T3: refused before anything changes | Compare again |
| Q4 | The pinned version loses its race after A's Pull finished | At the next refresh the storage reads `Displaced`, as R3 of the Push plan: nothing is rolled back. The bases and the acceptance this Pull saved stay and are read as absent. The installed sessions stay. Standing notice: "Another computer pushed to <storage> first. This computer's last pull is no longer part of its history. Compare again." | Compare again. The installed sessions read S2, S3 or S4 against the version that won |
| Q5 | B clears the storage between A's review and A's Pull | T4 or T5: refused before anything changes | Story 1 or story 2 |
| Q6 | B clears the storage while A's Pull applies | D0, D2 or D3: refused, or stopped with what it installed kept | N2 or N3 |
| Q7 | A starts a Push and a Pull at the same time on one computer | One operation runs at a time on one computer. The second one is refused: "another host-sensitive operation is already in progress" | As today |
| Q8 | Two windows of Mallard apply the same review | The second apply finds its sessions changed and fails them alone (T7) | As today |

Q3 and Q4 can only happen on a local folder. Usually inside the seconds in
which two computers write onto the same version, but a writer that paused
after its listing, or a replica that was offline, can cause them at any later
time (CAS-free plan, section 7.2); the report of Q4 does not depend on when.
On R2 and on Dropbox the claim lets a version have one child, so a pinned
version never loses, and the lookup of section 4.1 answers T4 and T5 alone.

### 2.8 Library

A Library Pull installs skills, plugins and pets. Its review pins a version of
one repository in the same way, and its apply already works resource by
resource: each one is downloaded and verified completely before anything local
changes, and a resource that fails, fails alone.

| # | Case | After | Today |
|---|---|---|---|
| G1 | Another computer pushed a newer version between the review and the Pull | The Pull applies the reviewed resources (rule 1). Resources the newer version changed read "changed in storage" afterwards | Refused: "the stored content changed after review" |
| G2 | The reviewed version lost its race, or the repository was cleared, before the Pull starts | Refused before anything changes: "The storage changed since this review. Review again." The page then resets the link, as B1 and B2 of the Push plan | Refused, with the text of G1, or with "the stored repository head disappeared" |
| G3 | The repository is cleared while the Pull applies, or a synced folder has not replicated a payload yet | The Pull stops at the first payload it cannot read (rule 3). The resources installed before stay, with their claims. The others read "Not pulled". The link's accepted version does not advance | Each remaining resource fails alone with "Library payload '<key>' is missing". The end state is the same; the result shows one failure per resource |
| G4 | One resource fails: its target changed here since the review, the provider's own installer failed, or the written files did not verify | It fails alone. The link's accepted version does not advance, and the resources that succeeded keep their own records | As today |
| G5 | The review is older than a day, or a newer review of the same row replaced it | "Review again" | As today |
| G6 | A reset or an unlink | Never offered | As today: no text in the Library Pull path asks for one |

The first case of G2 is Q3's: a Library record claims its parent the same way,
so on R2 and on Dropbox only a cleared repository refuses a Library Pull.

### 2.9 Cases this plan leaves as they are

| # | Case | What the user sees |
|---|---|---|
| K1 | A session's local file is gone, or cannot be read | "Can't sync" |
| K2 | A stored session was written with credential redaction rules, or a path encoding, that this version of Mallard does not carry | "Can't sync", with the reason. Aligning the app versions resolves it |
| K3 | A stored subagent rollout that was published as a task of its own | It is not offered by Pull |
| K4 | A stored file larger than the per-file limit | That row fails alone, with the limit |
| K5 | L3 and L4 of the Push plan | "Keep both" |
| K6 | A Pull that was interrupted by a crash in the middle of one session's files (D8) | The replaced files are in the backup folder. Nothing resumes |
| K7 | A "Mark synced" row in a review that also holds other rows | The row cannot be ticked, so that Pull does not save it. The next Pull whose review holds nothing else saves it without opening: "Storage reviewed for <project> — files unchanged." Until then the session reads S2 at every comparison, and Mallard proves it again each time |
| K8 | A project file the storage deleted, and a file with no safe target here (F5, F6) | The backend plans a row for each; the Pull review shows neither |

## 3. Why a Pull can apply the version it reviewed

**The pinned version cannot change under the Pull.** A record's name contains
the digest of its bytes, the record names its manifest by digest, and the
manifest names every object by digest. Whatever a computer pushes afterwards is
a new record, a new manifest and new objects. So reading the pinned version at
10:06 gives the same bytes as at 10:00, or it fails. It never gives something
else.

**An older version on the same history is safe to install.** Every later
version was built on it. For a session this holds because a stored session only
grows: the newer version's copy is the pinned copy plus more turns, so after the
Pull the row reads "Update this task". For a file the newer version holds a
newer content or a removal record, and the next Pull offers it. For a storage
area the saved base names the pinned version, which is on the history, so the
area reads "has changes" exactly when the newer version changed it, and Push
asks for a Pull exactly then.

**The second read of the head protects nothing.** The apply reads the head
twice today: before it starts, and again before it writes files. Both checks
were written when the head was one mutable file. Since the CAS-free cutover the
head is selected from immutable records, and the pinned bytes are the same
whoever pushed since. What the second read does today is turn D1 into an error
after the sessions were already installed.

**A version that lost its race is not pulled.** Its content is not part of the
history the computers agree on. The computer that pushed it still holds that
content and pushes it again. A Pull of it would be reported as displaced at the
next refresh (Q4). So
the apply asks for one thing before it starts: the pinned version is the
selected one or one of its ancestors. On R2 and on Dropbox no listed version
loses (section 2.4), and there the same question refuses only a cleared or
recreated history (T4, T5).

**Stopping is enough when the storage disappears.** Step 4 downloads and
verifies every ticked session before step 5 installs any of them, so a session
is never installed from a half-read copy. What was installed is a complete
session with a complete record. Rolling it back would delete something the user
asked for, and it is not needed: after a clear that record is stale, the Push
plan ignores it, and the session reads "Publish new task". Trying again is not
needed either: the objects are gone, and the next comparison shows what the
storage holds now. When the storage is a folder that has not finished
replicating, the objects arrive by themselves, the record is usable, and the
next comparison offers the rest.

**A reset never helps a Pull.** "Reset local sync state" is offered after one
Pull failure today: a stored session whose content does not match its manifest
(D6). The mismatch is in the storage, or between two versions of the app. The
local records play no part in it, so the same Pull fails again after the reset,
and the reset costs every other session its record. The way out is rule 5: leave
that row unticked.

Two risks are accepted:

- **A Pull can install a version that is already a little old.** Up to one day
  old, which is how long a review is valid. The next comparison shows the rest.
- **A clear during step 5 is not noticed until step 6.** The sessions are
  installed from their downloaded copies although the storage is already empty.
  Nothing is lost: they are N2 rows afterwards.

## 4. How it is built

### 4.1 The pin

The review already carries the pin: the version's revision, its commit id and
its manifest digest. No document and no payload changes shape.

The apply lists the history once and looks the pinned version up by revision and
commit id on the selected chain. When it is there, the apply reads that record
and that manifest by name, and checks the manifest digest against the pin. When
it is not there, the apply is refused (T3 to T5). It is also refused when a base
of this link names a later version than the pin (T11). This replaces the
equality between the selected version and the pinned one.

The selected chain of the same listing goes into the session review of step 2,
which reads saved acceptance through it (Push plan, section 4.3). The bases that
step 7 saves name the pinned version and take their content coordinates from the
pinned manifest, as they do today. The readiness panel beside the review reads
the pinned version as well, so the review shows one version everywhere.

### 4.2 One read of the head

The file step stops reading the head. It keeps comparing the plan with the
snapshot it is given, which is the pinned one.

### 4.3 Stopping

Steps 4 and 6 read objects through one byte source. It already tells a missing
object from a mismatching one and from a transport error. A missing object of a
published version means the storage was cleared, or, in a folder that a sync
product replicates, that the object has not arrived yet; nothing else removes
one, and Mallard cannot tell the two folders apart. The byte source reports it
with one fixed text that is true of both, as the publication module does for
its outcomes, and the text names no cause.

Today an error in step 4 or step 6 ends the whole call. After this plan it ends
the *work*: the apply keeps the receipts it has, writes a "Not pulled" receipt
for every ticked item it did not reach, saves the result as it does for a
finished Pull, does not mark the areas as reviewed, and returns the result with
the reason it stopped. Errors of steps 1 and 2 still refuse the Pull, because
nothing has changed yet.

Each installed session already saves its own receipt with its own local save, so
step 7 only adds to what is there. The result gains one optional field for the
reason; it is a command result, not a saved document.

### 4.4 No reset in the Pull flow

A reset is offered from three Pull surfaces: the review's error banner, the
storage card's "Pull couldn't start" notice, and the Pull result, where it
pushes every other action aside. One matcher decides all three, and the two
banners of the Push review use it too. It looks for three texts. Two of them
name a pin the short way, and the backend prints the pin's full name, so they
never match anything the app produces. The third is D6.

The matcher and its five uses are removed. D6 gets its own sentence and no
action. "Reset local sync state…" stays where the user asks for it: the project
menu.

### 4.5 Copy

Every per-session failure that means "this changed here since the review" prints
one sentence, whatever pin noticed it. The refusals of T3 to T6 and T11 and the
notice of O2 get the texts of section 5, and their banner gets its Compare again
button. The backend keeps its precise text in the log.

O2 is told from O1 by the link's bases: a computer that holds a base for this
storage has synced with it before.

### 4.6 The displaced notice after a Pull

The report of Q4 is the Push plan's `Displaced` state (CAS-free plan, section
9.4): nothing is rolled back, and the bases and the acceptance this Pull saved
are kept and read as absent. Its standing notice says "last push" or "last
pull" by which operation saved the newest stale base. A base already records
when it was last pushed and when it was last pulled.

### 4.7 Library

The Library apply observes the repository once, before it installs anything,
and compares the selected version with the pinned one for equality. That
becomes the lookup of section 4.1: the pinned version is on the selected chain.
The workspace that the apply resolves hands the chain out next to the selected
version.

Nothing else moves. The apply already takes every resource description from the
reviewed plan, never from a second read, and fetches payloads by their digest,
so it reads the pinned content whatever was pushed since. It saves the pinned
version as the link's accepted version, which is on the chain, so continuity
reads it as usable.

For G3 the apply stops at the first missing payload instead of failing every
remaining resource with the same text. A payload is fetched and verified before
its resource changes anything local, so a stop never leaves a resource half
installed.

## 5. What the user sees

| When | Where | Copy |
|---|---|---|
| O2 | the storage card's notice, in the note tone, with no button | "There is nothing to pull from <storage>." |
| T3, T4, T5, T11 | the Pull review's error banner, with Compare again | "The storage changed since this review. Compare again." |
| T6 | the same | "This review is older than a day. Compare again." |
| T7 and every other local change to one session | that row of the result | "This conversation changed after the comparison. Compare again." |
| D2, D3 | the Pull result, titled "Pull stopped" | "Part of this version is missing from <storage>. Compare again." |
| D4 | the same | the storage's own error, as the storage card prints it today |
| D2 to D4 | each row the Pull did not reach | "Not pulled" |
| D2 to D4 | the result's counts | "N restored · M not pulled" |
| D6 | that row of the result | "This stored session cannot be read by this version of Mallard." |
| D7 | that row of the result | "Sync settings changed while pulling. Pull again." |
| Q4 | the project activity notice zone, standing, without an action | "Another computer pushed to <storage> first. This computer's last pull is no longer part of its history. Compare again." |
| G2 | the Library Pull review | "The storage changed since this review. Review again." |
| G3 | the Library Pull result | "Part of this version is missing from <storage>. Compare again.", and "Not pulled" on the rows it did not reach |

A stopped Pull does not print "Push is still blocked". That line tells the user
to pull the missing changes, and after a clear there is nothing to pull. The
result's sentence names no cause, because Mallard cannot tell a cleared storage
from a folder still replicating: after a clear the standing notice of the Push
plan says what to do next, and in a folder still replicating the next
comparison shows the version whole. The result's action is "Open Pull review",
as for any Pull that left something out.

## 6. What is removed

```text
the equality of the selected version and the reviewed one at the start of Pull apply
the second read of the head in the file step, with its two error texts
"bundle head changed after comparison; compare again"
the equality of the selected version and the reviewed one in the Library Pull apply, with "the stored content changed after review"
isIrrecoverableSyncError, and the "Reset local sync state" action on the Pull review banner, the storage card notice, the Pull result and the two Push review banners
the reset_metadata action of the sync outcome
the part of the engine test that pins the second read of the head
the frontend tests that assert the reset action
```

Unchanged: every persisted document, the storage layout, the review and what it
pins, the one-day validity, the decision matrix, first-contact adoption, "Keep
both", the per-item isolation, the backups, what marks an area as reviewed, and
"Reset local sync state…" in the project menu.

## 7. Work plan

| Step | Work | Done when |
|---|---|---|
| 1. Checks first | Two behaviors the plan rests on: a version that is an ancestor of the selected one reads completely by its name; a session installs from its downloaded copy while the storage is empty | Each has a test that says so |
| 2. The pin | Section 4.1: the lookup on the chain, the read by name, the digest check; remove the equality; the readiness panel reads the pin | T2 applies. T3, T4 and T5 are refused before any local save |
| 3. One read | Section 4.2; rewrite the engine test | D1 finishes, and its result lists everything |
| 4. Stopping | Section 4.3: the fixed text for a missing object; steps 4 and 6 end the work, not the call; "Not pulled" receipts; the areas stay unreviewed | D2, D3 and D4 return a result, and nothing is rolled back |
| 5. No reset | Section 4.4 | No conditional surface offers a reset |
| 6. Copy | Sections 4.5 and 4.6, the table of section 5, Compare again on the banner. The new texts go into the static specimen first | The frontend suite asserts each text |
| 7. Library | Section 4.7: the lookup on the chain; stop at the first missing payload | G1 applies the reviewed resources. G2 is refused before any local change. G3 returns a result. The catalog suites pass |
| 8. Docs | Sections 9.3 and 12 of the CAS-free plan; `CURRENT_METADATA_LAYOUT.md`, `TECHNICAL_REPORT.md`, `README.md` | The documents describe this behavior |
| 9. Gate | `cargo test --workspace --all-targets`, `npx tsc --noEmit`, the frontend integration suite, `npm run build` | All pass |

## 8. Tests

Every changed row of section 2 gets a test that names it.

| Cases | Expected |
|---|---|
| T2, Q1 | A newer version is pushed after the review. The Pull applies the pinned version: the installed session has the pinned turns, the record names the pinned version, and the bases take the pinned manifest's coordinates. The next comparison reads S8 and A6 |
| T2, other areas only | The newer version changed only an area this computer does not write. After the Pull, Push is open |
| T3 | A record with an earlier stamp is planted beside the pinned one, directly in the store, because on R2 and Dropbox no Push can write it. The Pull is refused, and no local document changed |
| T4, T5 | The storage is wiped, and wiped and pushed again. The Pull is refused, and no local document changed |
| T11 | This computer pushes after the review. The older review is refused, and the bases still name the pushed version |
| D1, Q2 | A newer version is planted after step 1. The Pull finishes, and the result lists the installed sessions and the written files |
| D2 | The storage is wiped before step 4. Nothing is installed, the result is titled "Pull stopped" and prints "Part of this version is missing from <storage>. Compare again.", and the bases did not move |
| D3 | The storage is wiped after step 4. The sessions are installed, the files are not, every unreached item reads "Not pulled", and the bases did not move |
| D2, D3 in a folder still replicating | One object of the pinned version is removed before the Pull and put back after it. The Pull stops with the same result and no cause in its text. After the object is back, the next comparison offers the rest, and the installed sessions read in sync (N7) |
| N2 | After D3, the next comparison reads story 1: the installed sessions read "Publish new task" under their cloud ids, and one Push recreates the storage |
| D4 | The store fails every read after step 4. The result carries the store's error, and what was installed stays |
| D6 | A stored session whose content does not match its manifest fails alone, prints its sentence, and no surface offers a reset. With that row unticked, the next Pull marks the areas as reviewed |
| Q4 | A version that a Pull applied is outranked afterwards, by a record planted as in T3. The refresh reads `Displaced`, changes no base and no acceptance, keeps the installed sessions, and the notice says "last pull" |
| O2 | A Pull on a cleared, empty storage prints its notice and opens neither a review nor the chooser. O1 still opens the chooser |
| G1 | A newer Library version is pushed after the review. The Pull installs the reviewed resources and saves the pinned version as accepted. The next page reads the changed rows as "changed in storage" |
| G2 | The reviewed Library version is outranked, and the repository is cleared. Both are refused, and no local file and no link changed |
| G3 | The repository is removed after the first of two resources was installed. The first stays with its claim, the second reads "Not pulled", and the accepted version did not advance |
| Rewritten | The engine test of the second read of the head; the frontend tests of the reset action in the Pull review, the Pull result and the Push review |

## 9. Not in this plan

- **Resuming or retrying a stopped Pull.** The owner's rule: it stops, and it
  does not recover today.
- **Rolling back what a stopped Pull installed.** Section 3 gives the reason.
- **Finishing the interrupted session of D8.** The backups are there; nothing
  replays a multi-file install.
- **A review that follows the storage while it is open.** The review stays a
  snapshot. Rule 1 makes that harmless.
- **K7.** Its fix is small: every Pull sends the review's "Mark synced" rows
  along with the ticked ones, as the Pull without a review already does. It
  changes what a Pull approves, so it is a decision of its own.
- **K8.** Showing stored deletions and blocked files in the Pull review.
- **Making K2 pullable across app versions.**
- **Selecting the record that landed first, so that a published version could
  never lose (T3).** Only the storage knows when a record landed, and its answer
  is not usable for selection. It is one second coarse on S3 and on Dropbox, so
  two records that land in the same second still tie, and the tie can still turn
  over what a reader already saw. It is not part of the record's name, so a
  copied, migrated or restored storage gets new times and selects differently.
  And in a folder that a sync product replicates, every computer has its own
  arrival times, so two computers would select different versions. Today's rule
  reads names only, which is why every computer selects the same version from
  the same records, always. It would not shorten this plan either: T3 is decided
  by the lookup that a cleared storage needs anyway (T4, T5), and Q4 by the
  `Displaced` report that the Push plan already has.
- **The claim that makes a listed version final on R2 and Dropbox.** It is
  decision 11 and section 7.2 of the CAS-free plan: one create-only write under
  the parent version's name, before the record, which the storage refuses for
  the second computer. It is written there and not here because it is a store
  contract. It removes T3, Q3, Q4 and G2 on those storages and changes nothing
  in this plan: the lookup of section 4.1 is what a cleared storage needs anyway
  (T4, T5), and the report of Q4 is the Push plan's, which a local folder
  still needs. Google Drive and Feishu cannot refuse a second write, because two
  files there can carry one name, and a folder that a sync product replicates
  cannot either; on those the selector and the `Displaced` report remain the
  whole rule.
- **Shortening the usual window of T3 on a local folder.** A record is
  stamped before its manifest is uploaded. Stamping it right before
  list-then-write, and capping the time of the record request, would make the
  common case a few seconds instead of about a minute. It bounds nothing: a
  writer that pauses after its listing stays possible. It belongs to the
  CAS-free plan.

## Source pointers

Line numbers are those of the working tree on September 20, 2026.

| Subject | Location |
|---|---|
| The review: `plan_bundle_restore_with_progress`; "bundle '<id>' does not exist" | `src-tauri/src/project_sync/service.rs:10740`, `:10771` |
| The pin on the review: `revision`, `commit_id`, `manifest_sha256`, `expires_at` | `crates/mallard-domain/src/lib.rs:5286` |
| The review's lifetime | `crates/mallard-infrastructure/src/bundle_engine.rs:46` |
| The apply: `apply_bundle_restore_with_progress`; the equality with the selected version | `src-tauri/src/project_sync/service.rs:12602`, `:12617-12622` |
| `inspect_from_storage`: the snapshot with its chain | `src-tauri/src/project_sync/service.rs:3271` |
| `Mainline::contains`; `snapshot_at`; `observe` | `crates/mallard-domain/src/publication.rs:266`; `crates/mallard-infrastructure/src/bundle_engine.rs:746`, `:769` |
| Approval checks and the expiry text | `src-tauri/src/project_sync/service.rs:12486`, `:12493` |
| Binding and identity checks (T9) | `src-tauri/src/project_sync/service.rs:13812` |
| Step 2: `build_restore_plan_for_apply`; the stored-subagent read; aligning the ticked rows | `src-tauri/src/project_sync/service.rs:10883`, `:6292`, `:12528` |
| Step 3: `adopt_cloud_metadata_actions` | `src-tauri/src/project_sync/service.rs:13291` |
| Step 4: `prepare_pull_conversation_actions`; its batched download | `src-tauri/src/project_sync/service.rs:11077`, `:11137` |
| Step 5: the install loop; "sync config changed during Pull apply"; one session's install | `src-tauri/src/project_sync/service.rs:12809-12980`, `:12936`, `:11541` |
| One session's files, backup and local save; "not replayed, rolled back, or resumed" | `crates/mallard-infrastructure/src/conversation_materialization.rs:222`, `:1-6` |
| Step 6: the call; `apply_restore_plan_lazy`; `validate_plan_pin`; the second read of the head; the file download | `src-tauri/src/project_sync/service.rs:12991`; `crates/mallard-infrastructure/src/bundle_engine.rs:6109`, `:5725`, `:6182-6191`, `:6223` |
| The byte source: missing, mismatching, transport | `crates/mallard-infrastructure/src/bundle_engine.rs:317`, `:414-425` |
| Step 7: what marks the areas as reviewed; the bases and the stale ones; the result | `src-tauri/src/project_sync/service.rs:13472`, `:13085-13145`, `:865` |
| The per-session pins and their names (T7) | `crates/mallard-infrastructure/src/conversation_sync.rs:119`; `src-tauri/src/project_sync/service.rs:11311` |
| D6: "cloud conversation changed lineage or semantic content" | `crates/mallard-infrastructure/src/pull_transcript.rs:72`, `:118` |
| File rows of the review; the local copy of a diverged file | `crates/mallard-infrastructure/src/bundle_engine.rs:1392`, `:5040` |
| The report of Q4: `observe_linked_bundle`, today the rollback, to become the `Displaced` classifier (CAS-free plan, step 10) | `src-tauri/src/project_sync/service.rs:7092` |
| One operation at a time on one computer (Q7) | `crates/mallard-infrastructure/src/persistence.rs:380`, `:411` |
| The engine test that pins the second read of the head | `crates/mallard-infrastructure/src/bundle_engine.rs:4567`, `:4733-4752` |
| Existing Pull tests: per-session isolation, approvals, a row the apply no longer offers, an empty approval | `src-tauri/src/project_sync/service.rs:23832`, `:24133`, `:24393`, `:24192` |
| The two-computer fixture for the new tests | `src-tauri/src/project_sync/service.rs:21050` |
| Library: the review; the apply; the equality; the loop; when the accepted version advances | `src-tauri/src/storage_catalog/service.rs:1964`, `:2019`, `:2063`, `:2120`, `:2151-2180` |
| Library: payload fetch and its missing text; one resource's apply; the review's lifetime | `src-tauri/src/storage_catalog/resource_engine.rs:268`, `:280`; `src-tauri/src/storage_catalog/apply.rs:1134`; `src-tauri/src/storage_catalog/resource_planning.rs:29` |
| The Pull flow in the shell: review, the chooser of O1, apply, closing | `src/components/project-sync/ProjectSync.tsx:2207`, `:2379`, `:2417`, `:2809`, `:2582` |
| The apply error and its prefix; the Pull without a review | `src/components/project-sync/pullReviewFlow.ts:112`, `:136`, `:73` |
| The review's error banner; the footer's verb; Rescan | `src/components/project-sync/RestorePlanView.tsx:379`, `:281`, `:537` |
| The reset matcher and its uses | `src/components/project-sync/syncError.ts:6`; `RestorePlanView.tsx:382`; `ProjectLinksWorkspace.tsx:2911`; `syncOutcome.ts:289`, `:316`; `PushResourceWorkspace.tsx:741`, `:1017` |
| The Pull result: titles, "Push is still blocked", actions | `src/components/project-sync/syncOutcome.ts:192`, `:279`, `:315-341` |
| "Pull couldn't start" | `src/components/project-sync/ProjectLinksWorkspace.tsx:2894` |
| "Reset local sync state…" in the project menu | `src/components/project-sync/ProjectLinksWorkspace.tsx:2201` |
| Which session rows can be ticked (K7) | `src/components/project-sync/pullReviewModel.ts:35` |
| Row labels | `src/components/project-sync/ProjectActivityPage.tsx:1101-1139` |
