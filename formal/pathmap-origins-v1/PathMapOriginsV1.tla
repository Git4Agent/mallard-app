---- MODULE PathMapOriginsV1 ----
EXTENDS Naturals, Sequences, FiniteSets

(***************************************************************************
PROPOSAL, not an implementation model of the current Rust codec.

One sequence represents one normalized record's lexical path atoms. A real
implementation must also bind the origins to the record, thread, codec, and
typed JSON locations. Hash verification is abstracted as exact equality.
Literal occurrences remain data even when they equal the destination root.
***************************************************************************)

CONSTANTS MaxLen, MaxAppendLen

Roots == {"A", "B", "C"}
Raw(root) == CASE root = "A" -> "raw-A" [] root = "B" -> "raw-B" [] OTHER -> "raw-C"
Uri(root) == CASE root = "A" -> "uri-A" [] root = "B" -> "uri-B" [] OTHER -> "uri-C"
Plain == "plain"
TagRoot == "/r"
TagEscape == "/e"
LiteralRoot == "literal-root"
LiteralEscape == "literal-escape"
WireRaw == "wire-root-raw"
WireUri == "wire-root-uri"
WireEscape == "wire-escape"
StrayRoot == "stray-root"
Rejected == "rejected"

SourceAlphabet ==
  {Plain, TagRoot, TagEscape, LiteralRoot, LiteralEscape}
  \cup {Raw(root) : root \in Roots}
  \cup {Uri(root) : root \in Roots}
WireAlphabet ==
  {Plain, TagRoot, TagEscape, WireRaw, WireUri, WireEscape, StrayRoot}
  \cup {Raw(root) : root \in Roots}
  \cup {Uri(root) : root \in Roots}

RECURSIVE Words(_, _)
Words(alphabet, bound) ==
  IF bound = 0 THEN {<<>>}
  ELSE Words(alphabet, bound - 1)
       \cup {Append(word, atom) :
               word \in Words(alphabet, bound - 1), atom \in alphabet}

EncodeAtom(atom, root) ==
  CASE atom = Raw(root) -> <<WireRaw>>
    [] atom = Uri(root) -> <<WireUri>>
    [] atom = LiteralRoot -> <<WireEscape, TagRoot>>
    [] atom = LiteralEscape -> <<WireEscape, TagEscape>>
    [] OTHER -> <<atom>>

RECURSIVE Encode(_, _)
Encode(local, root) ==
  IF Len(local) = 0 THEN <<>>
  ELSE EncodeAtom(Head(local), root) \o Encode(Tail(local), root)

RECURSIVE WireOK(_)
WireOK(wire) ==
  IF Len(wire) = 0 THEN TRUE
  ELSE IF Head(wire) = StrayRoot THEN FALSE
  ELSE IF Head(wire) = WireEscape
       THEN Len(wire) >= 2 /\ wire[2] \in {TagRoot, TagEscape}
            /\ WireOK(SubSeq(wire, 3, Len(wire)))
       ELSE WireOK(Tail(wire))

Prepend(atom, origin, rest) ==
  [local |-> <<atom>> \o rest.local,
   origins |-> <<origin>> \o rest.origins]

RECURSIVE Decode(_, _)
Decode(wire, root) ==
  IF Len(wire) = 0 THEN [local |-> <<>>, origins |-> <<>>]
  ELSE IF Head(wire) = WireRaw
       THEN Prepend(Raw(root), "root-raw", Decode(Tail(wire), root))
  ELSE IF Head(wire) = WireUri
       THEN Prepend(Uri(root), "root-uri", Decode(Tail(wire), root))
  ELSE IF Head(wire) = WireEscape
       THEN IF wire[2] = TagRoot
            THEN Prepend(LiteralRoot, "literal-root",
                         Decode(SubSeq(wire, 3, Len(wire)), root))
            ELSE Prepend(LiteralEscape, "literal-escape",
                         Decode(SubSeq(wire, 3, Len(wire)), root))
       ELSE Prepend(Head(wire), "keep", Decode(Tail(wire), root))

CheckedDecode(wire, root) ==
  IF WireOK(wire) THEN Decode(wire, root) ELSE Rejected

ReplayAtom(atom, origin) ==
  CASE origin = "root-raw" -> <<WireRaw>>
    [] origin = "root-uri" -> <<WireUri>>
    [] origin = "literal-root" -> <<WireEscape, TagRoot>>
    [] origin = "literal-escape" -> <<WireEscape, TagEscape>>
    [] OTHER -> <<atom>>

RECURSIVE Replay(_, _)
Replay(local, origins) ==
  IF Len(local) = 0 THEN <<>>
  ELSE ReplayAtom(Head(local), Head(origins))
       \o Replay(Tail(local), Tail(origins))

RestoreWire(wire, root) ==
  LET decoded == Decode(wire, root)
  IN Replay(decoded.local, decoded.origins)

RECURSIVE Rebase(_, _, _)
Rebase(source, oldRoot, newRoot) ==
  IF Len(source) = 0 THEN <<>>
  ELSE <<CASE Head(source) = Raw(oldRoot) -> Raw(newRoot)
             [] Head(source) = Uri(oldRoot) -> Uri(newRoot)
             [] OTHER -> Head(source)>>
       \o Rebase(Tail(source), oldRoot, newRoot)

VARIABLES source, appended, wire, other, fault
vars == <<source, appended, wire, other, fault>>
Stay == UNCHANGED vars

SourceInit ==
  /\ source \in Words(SourceAlphabet, MaxLen)
  /\ appended = <<>> /\ wire = <<>> /\ other = <<>> /\ fault = "none"
SourceSpec == SourceInit /\ [][Stay]_vars

SourceWire == Encode(source, "A")
OnB == Decode(SourceWire, "B")
Replayed == Replay(OnB.local, OnB.origins)

OriginLengthMatches == Len(OnB.local) = Len(OnB.origins)
SourceIdentity == Decode(SourceWire, "A").local = source
OriginReplayIsLossless == Replayed = SourceWire
LiteralPathsStayLiteral == OnB.local = Rebase(source, "A", "B")
ThirdMachineKeepsMeaning ==
  Decode(Replayed, "C").local = Rebase(source, "A", "C")
SecondReplayIsLossless == RestoreWire(Replayed, "C") = SourceWire

(***************************************************************************
Negative control: removing the current gate and doing ordinary root scanning
on the next Push loses the distinction between tokens and literal paths.
***************************************************************************)
BlindReplayIsLossless == Encode(OnB.local, "B") = SourceWire

(***************************************************************************
Negative control: an empty inverse-edit list is not evidence of a new record.
Imported records with no token edits still need provenance coverage.
***************************************************************************)
HasInverseEdits == \E index \in 1..Len(OnB.origins) : OnB.origins[index] # "keep"
EmptyTraceFallbackIsSafe ==
  (IF HasInverseEdits THEN Replayed ELSE Encode(OnB.local, "B")) = SourceWire

ContinuationInit ==
  /\ source \in Words(SourceAlphabet, MaxLen)
  /\ appended \in Words(SourceAlphabet, MaxAppendLen)
  /\ wire = <<>> /\ other = <<>> /\ fault = "none"
ContinuationSpec == ContinuationInit /\ [][Stay]_vars
ExtendedWire == Replayed \o Encode(appended, "B")
AcceptedPrefixDoesNotChange ==
  SubSeq(ExtendedWire, 1, Len(SourceWire)) = SourceWire
AppendedHistoryKeepsMeaning ==
  Decode(ExtendedWire, "C").local =
    Rebase(source, "A", "C") \o Rebase(appended, "B", "C")
ExtendedReplayIsLossless == RestoreWire(ExtendedWire, "C") = ExtendedWire

WireInit ==
  /\ wire \in Words(WireAlphabet, MaxLen)
  /\ source = <<>> /\ appended = <<>> /\ other = <<>> /\ fault = "none"
WireSpec == WireInit /\ [][Stay]_vars
ValidWireRoundTrips ==
  IF WireOK(wire) THEN RestoreWire(wire, "B") = wire ELSE TRUE
MalformedWireRejected == ~WireOK(wire) => CheckedDecode(wire, "B") = Rejected

(***************************************************************************
Integrity and ownership guards are checks on a record captured once for the
attempt. They are not extra reads of a live provider transcript. Provenance
files are hash-bound to a committed reference. Hash collisions are excluded.
***************************************************************************)
Faults == {"none", "missing", "uncovered", "wrong-local", "wrong-origins", "wrong-owner"}
GuardInit ==
  /\ source \in Words(SourceAlphabet, MaxLen)
  /\ fault \in Faults
  /\ appended = <<>> /\ wire = <<>> /\ other = <<>>
GuardSpec == GuardInit /\ [][Stay]_vars
ReadLocal == IF fault = "wrong-local" THEN Append(OnB.local, Plain) ELSE OnB.local
ReadOrigins ==
  IF fault = "wrong-origins" THEN Append(OnB.origins, "keep") ELSE OnB.origins
OriginGuard ==
  /\ fault # "missing" /\ fault # "uncovered" /\ fault # "wrong-owner"
  /\ ReadLocal = OnB.local /\ ReadOrigins = OnB.origins
GuardedPublication ==
  IF OriginGuard THEN Replay(ReadLocal, ReadOrigins) ELSE Rejected
UnprovenOriginDataRejected == fault # "none" => GuardedPublication = Rejected
VerifiedOriginDataPublishes == fault = "none" => GuardedPublication = SourceWire

(***************************************************************************
Two distinct JSON keys may decode to one key. Origins cannot make such an
object representable in native JSON. Keep the key collision rejection.
***************************************************************************)
PairInit ==
  /\ source \in Words(SourceAlphabet, MaxLen)
  /\ other \in Words(SourceAlphabet, MaxLen)
  /\ appended = <<>> /\ wire = <<>> /\ fault = "none"
PairSpec == PairInit /\ [][Stay]_vars
OtherWire == Encode(other, "A")
OtherOnB == Decode(OtherWire, "B")
OriginsPreserveDistinction ==
  (OnB.local = OtherOnB.local /\ OnB.origins = OtherOnB.origins)
    => SourceWire = OtherWire
CanInstallKeys == SourceWire # OtherWire /\ OnB.local # OtherOnB.local
InstalledKeyCount == Cardinality({OnB.local, OtherOnB.local})
InstalledKeysDoNotMerge == CanInstallKeys => InstalledKeyCount = 2
UnsafeKeyInstallIsLossless == SourceWire # OtherWire => InstalledKeyCount = 2

====
