# Live two-machine pass: R2, Dropbox, and Feishu

- Status: To be run once, by the owner, on two computers. It closes the live rows of `DESIGN_CAS_FREE_STORAGE_REFACTOR.md` (step 8 and step 9), `PLAN_CLEARED_STORAGE_PUSH_AGAIN.md` and `PLAN_PULL_PINNED_VERSION.md` in one wipe-and-retest, at commit 77b0d68 or later
- Written: September 22, 2026
- Record the result in the table at the end, then change the three plans' status lines from "still to be run and recorded" to the date

Every sentence in quotation marks below is the exact text the app prints. A
row passes when the app prints that text and the storage holds what the row
says. Nothing in this pass needs a reset, an unlink, or a hand edit of
`~/.mallard`.

## 0. Setup

1. On both computers, quit Mallard and delete `~/.mallard`.
2. Empty the R2 bucket and delete the Dropbox folder used for the test, or
   create fresh ones. Both storages are set up again below, so the setup
   probes run with their new refusal stage.
3. On computer A: add the R2 storage and the Dropbox storage. Both setups
   must pass. A setup that fails at "refusal of a second write" (R2) or with
   a cleanup error (Dropbox) means the destination overwrites, and the
   storage cannot be used for claims.
4. On computer A: register one project with a few Codex or Claude sessions
   and a few project files; link it to both storages.
5. On computer B: add the same two storages (discover the existing ones),
   link the same project to both.

Run parts 1 to 5 on R2 first, then on Dropbox. In each part "the storage"
means the one being tested.

## 1. Ordinary publication (CAS-free step 8)

| # | Do | Expect |
|---|---|---|
| 1.1 | A: Push | "Push complete". The storage holds `_commits/<bundle>/` with one record, `_manifests/`, `_objects/`, `_claims/<bundle>/root.json`, and no `_head.json`, no `_tags/` |
| 1.2 | B: Compare, Pull | The Pull installs A's sessions. B's storage card reads in sync afterwards |
| 1.3 | B: change a session, Push | "Push complete". `_commits/<bundle>/` holds two records, `_claims/<bundle>/` holds two objects: `root.json` and `<first record's token>.json` |
| 1.4 | A: Compare | The storage reads ahead ("Pull required" for the changed area); Pull, then the card reads in sync |
| 1.5 | A: Compare again with nothing changed | The card still reads in sync. Optional, from the bucket's request metrics: one listing request per Compare and no object read for the unchanged selection |

## 2. Simultaneous pushes: one child per version (CAS-free step 9)

| # | Do | Expect |
|---|---|---|
| 2.1 | Both computers: change a different session each, open the Push review on both, then press Push on both within the same two seconds | One of them: "Push complete". The other: "The storage changed while pushing. Compare again." No third sentence |
| 2.2 | Inspect the storage | The version both pushed on has exactly one child record under `_commits/<bundle>/`, and exactly one claim object for it under `_claims/<bundle>/`. The losing computer wrote no record and no claim |
| 2.3 | The losing computer: Compare | The storage reads ahead; Pull, then Push again: "Push complete", and the storage holds one child of the new version |

## 3. A push that did not finish (CAS-free step 10)

Reproduce a writer that died between its claim and its record by removing
the record by hand and keeping the claim.

| # | Do | Expect |
|---|---|---|
| 3.1 | A: Push a change ("Push complete"). In the R2 dashboard or the Dropbox web app, delete the newest object under `_commits/<bundle>/` and leave `_claims/<bundle>/` untouched | The storage now holds a claim whose record is missing |
| 3.2 | B: Compare, then Push a change | The Push review banner reads "A push to <storage> did not finish. Finish it, then compare again." with the button **Finish that push**. Nothing was written: `_commits/<bundle>/` is unchanged |
| 3.3 | B: press Finish that push | The banner reads "Finished. Compare again." in the done tone. The deleted record is back under `_commits/<bundle>/`, byte for byte the claim's content |
| 3.4 | B: Compare | The storage reads ahead with A's version; Pull, then Push: "Push complete" |
| 3.5 | A: Compare | The storage reads ahead by B's push, and the sessions A pushed read synchronized, not held: A's own version is on the history again through B's finish. Pull, then the card reads in sync |
| 3.6 | Optional, the own-claim path. Repeat 3.1 on A. Then on A: Compare, take the Pull it asks for, and Push with no local change | "Push complete", and the storage's newest record is the one deleted in 3.1, under its old name: A met its own claim for the same content and wrote that record instead of a new one. Repeat 3.1 once more and change a session on A before the Push: "This computer's earlier push to <storage> did not finish. Finish it, then compare again." with the button |

## 4. A cleared storage (cleared plan, stories 1 and 2, R5)

| # | Do | Expect |
|---|---|---|
| 4.1 | A: Clear storage data (with sync stopped on B) | The storage keeps `.mallard/_storage.json` and nothing under `repositories/` or `library/`; A's card reads that the storage is empty |
| 4.2 | B: Compare | The standing note "Another computer cleared <storage>. Push again." Push: "Push complete", and the storage holds a first version again with `_claims/<bundle>/root.json` |
| 4.3 | A: Compare | The standing note "Another computer cleared <storage>. Pull, then push again." Pull, then Push: "Push complete" |
| 4.4 | A: start a Push; on B, Clear storage data while A's upload runs | A: "The storage changed while pushing. Compare again." or "Push complete" followed, at the next Compare, by the cleared-storage note. Either way no reset is offered, and the next Compare and Push on A go through |

## 5. Pull of a pinned version (Pull plan)

| # | Do | Expect |
|---|---|---|
| 5.1 | A: Push twice (two versions). B: open the Pull review, then A pushes a third version, then B applies the Pull | The Pull applies the reviewed version, not the third: "Pull complete" and the card then reads ahead by one version. No "changed since review" refusal |
| 5.2 | B: open the Pull review; A: Clear storage data; B: apply | "The storage changed since this review. Compare again." with the button **Compare again**. Nothing installed |
| 5.3 | A: Push again. B: open the Pull review, tick two sessions and one file; A: Clear storage data while B's download runs; B waits for the result | "Pull stopped": "Part of this version is missing from <storage>. Compare again." The sessions installed before the stop stay; the rest read "Not pulled"; no reset is offered |
| 5.4 | B: Compare | "Another computer cleared <storage>. Push again." Push, then A pulls |
| 5.5 | Library: link a skills scope on both; A pushes; B opens the Library Pull review; A pushes another skill; B applies | The reviewed version is applied ("Pull complete"), and the page then reads ahead |

## 6. Feishu Drive (PLAN_FEISHU_DRIVE_STORAGE step 6)

Two users of one Feishu tenant, Mallard's app available to both, a build
that carries the app, one Drive folder shared between them in Feishu. Run parts 1 and 5 on a Feishu
storage as written. Part 2 expects a different outcome: Feishu holds no
claims, so both records land and "displaced" is what the next Compare says
on the computer whose version lost. Part 3 does not apply (no claim, no
unfinished push). Part 4 does not apply (Clear storage data is unavailable).

- 6.1 Computer A: New storage → Feishu; Create new; Connect Feishu, sign in
  in a browser, nothing typed; Continue on the Account line;
  keep the proposed folder name; Create & connect; Test connection;
  Finish setup. The storage page prints "Sign in again by" with a date about
  a year out.
- 6.2 Computer B: share the folder in Feishu with user B, then New storage →
  Feishu; Use existing; Connect Feishu, sign in as user B; paste the
  folder's link; Use & connect; Test connection; Finish
  setup. The storage's folder token on both storage pages is the same.
- 6.3 Part 1 (ordinary publication) on this storage, A then B.
- 6.4 Part 2 (simultaneous pushes): both records land under `_commits`;
  the loser's next Compare reports the displaced version; a Push again from
  the loser builds on the winner.
- 6.5 Leave computer A closed past one access token's life (over two hours),
  then Push: the sign-in renews by itself; the storage page never asked to
  reconnect, and `~/.mallard/feishu_connections/<id>.json` holds a new
  `refresh_token` with `refresh_uncertain: false`.
- 6.6 On computer A, make Mallard's app unavailable to user A in Feishu (or
  revoke the sign-in), then Compare: "Feishu rejected the saved sign-in.
  Reconnect Feishu in Storage settings." with the route, status, code, and
  request ID; the sidebar marks the storage Reconnect; Reconnect Feishu on
  the storage page, then Save credential; Compare again succeeds. Computer B is
  unaffected throughout.
- 6.7 Part 5 (Pull of a pinned version) on this storage.

## Record

| Row | R2 | Dropbox | Feishu | Notes |
|---|---|---|---|---|
| 1.1 to 1.5 | | | | |
| 2.1 to 2.3 | | | (displaced, no claim) | |
| 3.1 to 3.6 | | | n/a | |
| 4.1 to 4.4 | | | n/a | |
| 5.1 to 5.5 | | | | |
| 6.1 to 6.7 | n/a | n/a | | |

Date run: · Commit: · Computers:
