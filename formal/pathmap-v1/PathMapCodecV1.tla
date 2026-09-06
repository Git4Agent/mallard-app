---- MODULE PathMapCodecV1 ----
EXTENDS Naturals, Sequences, FiniteSets

(***************************************************************************
The full-replacement mallard.pathmap.v1 grammar, modeled as lexical atoms.

The lexer must split every literal marker occurrence into LiteralRoot or
LiteralEscape.  TagRoot and TagEscape are ordinary source text unless they
immediately follow WireEscape in wire data.
***************************************************************************)

CONSTANT MaxSourceLen, MaxWireLen

Plain         == "plain"
TagRoot       == "/r"
TagEscape     == "/e"
SourceRaw     == "source-raw-path"
SourceUri     == "source-file-uri"
TargetRaw     == "target-raw-path"
TargetUri     == "target-file-uri"
LiteralRoot   == "literal-/__mallard_root__"
LiteralEscape == "literal-/__mallard_escape__"

WireRootRaw   == "wire-/__mallard_root__"
WireRootUri   == "wire-file:///__mallard_root__"
WireEscape    == "wire-/__mallard_escape__"
StrayRoot     == "boundary-invalid-/__mallard_root__"
Invalid       == "invalid-wire-escape"

SourceAlphabet == {
  Plain, TagRoot, TagEscape,
  SourceRaw, SourceUri,
  TargetRaw, TargetUri,
  LiteralRoot, LiteralEscape
}

WireAlphabet == {
  Plain, TagRoot, TagEscape,
  TargetRaw, TargetUri,
  WireRootRaw, WireRootUri, WireEscape, StrayRoot
}

RECURSIVE Words(_, _)
Words(alphabet, maxLen) ==
  IF maxLen = 0
  THEN {<<>>}
  ELSE Words(alphabet, maxLen - 1)
       \cup {Append(word, atom) :
               word \in Words(alphabet, maxLen - 1),
               atom \in alphabet}

SourceWords == Words(SourceAlphabet, MaxSourceLen)
WireWords   == Words(WireAlphabet, MaxWireLen)

Elems(sequence) == {sequence[index] : index \in 1..Len(sequence)}

EncodeSourceAtom(atom) ==
  CASE atom = SourceRaw     -> <<WireRootRaw>>
    [] atom = SourceUri     -> <<WireRootUri>>
    [] atom = LiteralRoot   -> <<WireEscape, TagRoot>>
    [] atom = LiteralEscape -> <<WireEscape, TagEscape>>
    [] OTHER                -> <<atom>>

EncodeTargetAtom(atom) ==
  CASE atom = TargetRaw     -> <<WireRootRaw>>
    [] atom = TargetUri     -> <<WireRootUri>>
    [] atom = LiteralRoot   -> <<WireEscape, TagRoot>>
    [] atom = LiteralEscape -> <<WireEscape, TagEscape>>
    [] OTHER                -> <<atom>>

RECURSIVE EncodeSource(_)
EncodeSource(source) ==
  IF Len(source) = 0
  THEN <<>>
  ELSE EncodeSourceAtom(Head(source)) \o EncodeSource(Tail(source))

RECURSIVE EncodeTarget(_)
EncodeTarget(source) ==
  IF Len(source) = 0
  THEN <<>>
  ELSE EncodeTargetAtom(Head(source)) \o EncodeTarget(Tail(source))

RECURSIVE DecodeAt(_, _, _)
DecodeAt(wire, rawRoot, uriRoot) ==
  IF Len(wire) = 0
  THEN <<>>
  ELSE IF Head(wire) = WireEscape
       THEN IF Len(wire) >= 2 /\ wire[2] = TagRoot
            THEN <<LiteralRoot>> \o DecodeAt(SubSeq(wire, 3, Len(wire)), rawRoot, uriRoot)
            ELSE IF Len(wire) >= 2 /\ wire[2] = TagEscape
                 THEN <<LiteralEscape>> \o DecodeAt(SubSeq(wire, 3, Len(wire)), rawRoot, uriRoot)
                 ELSE <<Invalid>>
       ELSE IF Head(wire) = WireRootRaw
            THEN <<rawRoot>> \o DecodeAt(Tail(wire), rawRoot, uriRoot)
            ELSE IF Head(wire) = WireRootUri
                 THEN <<uriRoot>> \o DecodeAt(Tail(wire), rawRoot, uriRoot)
                 ELSE IF Head(wire) = StrayRoot
                      THEN <<LiteralRoot>> \o DecodeAt(Tail(wire), rawRoot, uriRoot)
                 ELSE <<Head(wire)>> \o DecodeAt(Tail(wire), rawRoot, uriRoot)

DecodeSource(wire) == DecodeAt(wire, SourceRaw, SourceUri)
DecodeTarget(wire) == DecodeAt(wire, TargetRaw, TargetUri)

RECURSIVE WellFormedWire(_)
WellFormedWire(wire) ==
  IF Len(wire) = 0
  THEN TRUE
  ELSE IF Head(wire) = WireEscape
       THEN Len(wire) >= 2
            /\ wire[2] \in {TagRoot, TagEscape}
            /\ WellFormedWire(SubSeq(wire, 3, Len(wire)))
       ELSE WellFormedWire(Tail(wire))

RECURSIVE Rebase(_)
Rebase(source) ==
  IF Len(source) = 0
  THEN <<>>
  ELSE <<CASE Head(source) = SourceRaw -> TargetRaw
             [] Head(source) = SourceUri -> TargetUri
             [] OTHER -> Head(source)>>
       \o Rebase(Tail(source))

TargetCollisionFree(source) ==
  TargetRaw \notin Elems(source) /\ TargetUri \notin Elems(source)

VARIABLES source, left, right, wire

vars == <<source, left, right, wire>>

SourceInit ==
  /\ source \in SourceWords
  /\ left = <<>>
  /\ right = <<>>
  /\ wire = <<>>
SourceNext == UNCHANGED vars
SourceSpec == SourceInit /\ [][SourceNext]_vars

TypeOK == source \in SourceWords
SourceIdentity == DecodeSource(EncodeSource(source)) = source
CrossRootRebase == DecodeTarget(EncodeSource(source)) = Rebase(source)
EncodedWireIsWellFormed == WellFormedWire(EncodeSource(source))
TargetGateIsExact ==
  (EncodeTarget(DecodeTarget(EncodeSource(source))) = EncodeSource(source))
  <=> TargetCollisionFree(source)

PairInit ==
  /\ source = <<>>
  /\ left \in SourceWords
  /\ right \in SourceWords
  /\ wire = <<>>
PairNext == UNCHANGED vars
PairSpec == PairInit /\ [][PairNext]_vars

CodecInjective == (EncodeSource(left) = EncodeSource(right)) => (left = right)

(***************************************************************************
Mutation control: a literal root marker is emitted as the path control token.
Source identity must fail for a source that contains LiteralRoot.
***************************************************************************)
BrokenEncodeAtom(atom) ==
  CASE atom = SourceRaw     -> <<WireRootRaw>>
    [] atom = SourceUri     -> <<WireRootUri>>
    [] atom = LiteralRoot   -> <<WireRootRaw>>
    [] atom = LiteralEscape -> <<WireEscape, TagEscape>>
    [] OTHER                -> <<atom>>

RECURSIVE BrokenEncode(_)
BrokenEncode(sourceValue) ==
  IF Len(sourceValue) = 0
  THEN <<>>
  ELSE BrokenEncodeAtom(Head(sourceValue)) \o BrokenEncode(Tail(sourceValue))

BrokenSourceIdentity == DecodeSource(BrokenEncode(source)) = source

WireInit ==
  /\ source = <<>>
  /\ left = <<>>
  /\ right = <<>>
  /\ wire \in WireWords
WireNext == UNCHANGED vars
WireSpec == WireInit /\ [][WireNext]_vars

InvalidEscapeRejected ==
  (Invalid \notin Elems(DecodeTarget(wire))) <=> WellFormedWire(wire)

CanonicalForTarget(wireValue) ==
  /\ WellFormedWire(wireValue)
  /\ StrayRoot \notin Elems(wireValue)
  /\ TargetRaw \notin Elems(wireValue)
  /\ TargetUri \notin Elems(wireValue)

WireGateAccepts(wireValue) ==
  LET decoded == DecodeTarget(wireValue)
  IN /\ Invalid \notin Elems(decoded)
     /\ EncodeTarget(decoded) = wireValue

WireGateIsExact == WireGateAccepts(wire) <=> CanonicalForTarget(wire)

====
