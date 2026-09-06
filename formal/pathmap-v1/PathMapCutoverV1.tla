---- MODULE PathMapCutoverV1 ----
EXTENDS Naturals

(***************************************************************************
Clean in-place cutover.  The schema number and codec label stay unchanged.
No old state or old binary may coexist with enabled synchronization.
***************************************************************************)

Phases == {"stopped", "cleared", "installed", "enabled"}

VARIABLES phase, localOld, cloudOld, oldBinary, newBinary, syncEnabled

Init ==
  /\ phase = "stopped"
  /\ localOld = TRUE
  /\ cloudOld = TRUE
  /\ oldBinary = TRUE
  /\ newBinary = FALSE
  /\ syncEnabled = FALSE

Clear ==
  /\ phase = "stopped"
  /\ phase' = "cleared"
  /\ localOld' = FALSE
  /\ cloudOld' = FALSE
  /\ oldBinary' = FALSE
  /\ UNCHANGED <<newBinary, syncEnabled>>

Install ==
  /\ phase = "cleared"
  /\ phase' = "installed"
  /\ newBinary' = TRUE
  /\ UNCHANGED <<localOld, cloudOld, oldBinary, syncEnabled>>

Enable ==
  /\ phase = "installed"
  /\ phase' = "enabled"
  /\ syncEnabled' = TRUE
  /\ UNCHANGED <<localOld, cloudOld, oldBinary, newBinary>>

Stay == UNCHANGED <<phase, localOld, cloudOld, oldBinary, newBinary, syncEnabled>>

Next == Clear \/ Install \/ Enable \/ Stay
Spec == Init /\ [][Next]_<<phase, localOld, cloudOld, oldBinary, newBinary, syncEnabled>>

(***************************************************************************
Mutation control: synchronization can be enabled directly from stopped.
The legacy-data and legacy-execution invariants must reject this transition.
***************************************************************************)
UnsafeEnable ==
  /\ phase = "stopped"
  /\ phase' = "enabled"
  /\ syncEnabled' = TRUE
  /\ UNCHANGED <<localOld, cloudOld, oldBinary, newBinary>>

UnsafeNext == Next \/ UnsafeEnable
UnsafeSpec ==
  Init /\ [][UnsafeNext]_<<phase, localOld, cloudOld, oldBinary, newBinary, syncEnabled>>

TypeOK ==
  /\ phase \in Phases
  /\ localOld \in BOOLEAN
  /\ cloudOld \in BOOLEAN
  /\ oldBinary \in BOOLEAN
  /\ newBinary \in BOOLEAN
  /\ syncEnabled \in BOOLEAN

NoLegacyExecution == syncEnabled => ~oldBinary
NoLegacyData == syncEnabled => (~localOld /\ ~cloudOld)
OnlyNewBuildSyncs == syncEnabled => newBinary
EnabledOnlyAfterInstall == syncEnabled => phase = "enabled"

====
