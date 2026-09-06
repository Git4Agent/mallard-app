---- MODULE PathMapOriginCommitV1 ----
EXTENDS Naturals, FiniteSets

(***************************************************************************
Abstract per-thread best-effort Apply order. Revision 0 is the old pair;
revision 1 is the new pair. Origins are staged before provider installation.
The binding reference and accepted checkpoint use separate atomic writes.
A crash or write failure can leave provider bytes changed. There is no
rollback or automatic replay. A strict origin hash blocks stale re-encoding.
This model assumes a successful origin hash comparison is sound. It does not
model the OS, fsync, hash collisions, shared provider files, or Rust code.
***************************************************************************)
CONSTANT Threads
Phases == {"pending", "staged", "bytes-written", "bound", "accepted", "applied", "failed"}
VARIABLES phase, staged, transcript, origins, accepted, appliedEver
vars == <<phase, staged, transcript, origins, accepted, appliedEver>>

Init ==
  /\ phase = [thread \in Threads |-> "pending"]
  /\ staged = [thread \in Threads |-> FALSE]
  /\ transcript = [thread \in Threads |-> 0]
  /\ origins = [thread \in Threads |-> 0]
  /\ accepted = [thread \in Threads |-> 0]
  /\ appliedEver = {}

Stage(thread) ==
  /\ phase[thread] = "pending"
  /\ phase' = [phase EXCEPT ![thread] = "staged"]
  /\ staged' = [staged EXCEPT ![thread] = TRUE]
  /\ UNCHANGED <<transcript, origins, accepted, appliedEver>>

WriteTranscript(thread) ==
  /\ phase[thread] = "staged"
  /\ phase' = [phase EXCEPT ![thread] = "bytes-written"]
  /\ transcript' = [transcript EXCEPT ![thread] = 1]
  /\ UNCHANGED <<staged, origins, accepted, appliedEver>>

WriteBinding(thread) ==
  /\ phase[thread] = "bytes-written"
  /\ phase' = [phase EXCEPT ![thread] = "bound"]
  /\ origins' = [origins EXCEPT ![thread] = 1]
  /\ UNCHANGED <<staged, transcript, accepted, appliedEver>>

Accept(thread) ==
  /\ phase[thread] = "bound"
  /\ phase' = [phase EXCEPT ![thread] = "accepted"]
  /\ accepted' = [accepted EXCEPT ![thread] = 1]
  /\ UNCHANGED <<staged, transcript, origins, appliedEver>>

Receipt(thread) ==
  /\ phase[thread] = "accepted"
  /\ phase' = [phase EXCEPT ![thread] = "applied"]
  /\ appliedEver' = appliedEver \cup {thread}
  /\ UNCHANGED <<staged, transcript, origins, accepted>>

FailOrCrash(thread) ==
  /\ phase[thread] \notin {"applied", "failed"}
  /\ phase' = [phase EXCEPT ![thread] = "failed"]
  /\ UNCHANGED <<staged, transcript, origins, accepted, appliedEver>>

Next == \E thread \in Threads :
  Stage(thread) \/ WriteTranscript(thread) \/ WriteBinding(thread)
  \/ Accept(thread) \/ Receipt(thread) \/ FailOrCrash(thread)
Spec == Init /\ [][Next]_vars /\ WF_vars(Next)

TypeOK ==
  /\ phase \in [Threads -> Phases]
  /\ staged \in [Threads -> BOOLEAN]
  /\ transcript \in [Threads -> {0, 1}]
  /\ origins \in [Threads -> {0, 1}]
  /\ accepted \in [Threads -> {0, 1}]
  /\ appliedEver \subseteq Threads
AcceptedOnlyAfterCompleteInstall ==
  \A thread \in Threads : accepted[thread] = 1 =>
    (staged[thread] /\ transcript[thread] = 1 /\ origins[thread] = 1)
StagedBeforeVisibleWrite ==
  \A thread \in Threads : transcript[thread] = 1 => staged[thread]
SuccessfulSiblingsStayApplied ==
  \A thread \in appliedEver : phase[thread] = "applied" /\ accepted[thread] = 1
AllSettled == \A thread \in Threads : phase[thread] \in {"applied", "failed"}
EventuallySettled == <>AllSettled

UnsafeCommit(thread) ==
  /\ phase[thread] = "bytes-written"
  /\ phase' = [phase EXCEPT ![thread] = "accepted"]
  /\ accepted' = [accepted EXCEPT ![thread] = 1]
  /\ UNCHANGED <<staged, transcript, origins, appliedEver>>
UnsafeNext == Next \/ (\E thread \in Threads : UnsafeCommit(thread))
UnsafeSpec == Init /\ [][UnsafeNext]_vars
====
