---- MODULE PathMapSpellingV1 ----
EXTENDS Naturals, Sequences, FiniteSets

(***************************************************************************
PROPOSAL. Path references retain the matched source spelling. The companion
origin model checks marker escaping, provenance guards, and append behavior.
This model adds spelling preservation and return to the source machine.

Literal atoms stand for unchanged text, including marker-like text, Unicode,
quotes, controls, and redaction placeholders. They do not enumerate Unicode.
Input is after the separately approved credential-redaction exception.
***************************************************************************)

CONSTANT MaxLen
Machines == {"A", "B", "C"}
Styles == {"binding", "canonical", "tilde-binding", "tilde-canonical",
           "uri-raw-binding", "uri-raw-canonical",
           "uri-encoded-binding", "uri-encoded-canonical"}
LiteralLabels == {"plain", "root-marker", "escape-marker", "quotes-and-slashes",
                  "Unicode", "control", "redaction-placeholder"}
Literal(label) == [kind |-> "literal", text |-> label]
LocalPath(machine, style) == [kind |-> "local-path", machine |-> machine, style |-> style]
Reference(style) == [kind |-> "reference", style |-> style]

(***************************************************************************
B has no distinct canonical alias and no tilde spelling. Rendering uses an
available absolute spelling there but retains the original style in origins.
A and C have all eight distinct spellings. Machine bindings remain fixed.
***************************************************************************)
AvailableStyle(machine, style) ==
  IF machine # "B" THEN style
  ELSE CASE style \in {"binding", "canonical", "tilde-binding", "tilde-canonical"}
              -> "binding"
         [] style \in {"uri-raw-binding", "uri-raw-canonical"}
              -> "uri-raw-binding"
         [] OTHER -> "uri-encoded-binding"

Render(machine, style) == LocalPath(machine, AvailableStyle(machine, style))
SourceAlphabet == {Literal(label) : label \in LiteralLabels}
                  \cup {Render(machine, style) : machine \in Machines, style \in Styles}

RECURSIVE Words(_)
Words(bound) ==
  IF bound = 0 THEN {<<>>}
  ELSE Words(bound - 1)
       \cup {Append(word, atom) : word \in Words(bound - 1), atom \in SourceAlphabet}

EncodeAtom(atom) ==
  IF atom.kind = "local-path"
  THEN IF atom.machine = "A" THEN Reference(atom.style) ELSE atom
  ELSE atom

RECURSIVE Encode(_)
Encode(source) ==
  IF Len(source) = 0 THEN <<>>
  ELSE <<EncodeAtom(Head(source))>> \o Encode(Tail(source))

RECURSIVE Decode(_, _)
Decode(wire, machine) ==
  IF Len(wire) = 0 THEN [local |-> <<>>, origins |-> <<>>]
  ELSE LET atom == Head(wire)
           rest == Decode(Tail(wire), machine)
           isReference == atom.kind = "reference"
       IN [local |-> <<IF isReference THEN Render(machine, atom.style) ELSE atom>>
                      \o rest.local,
           origins |-> <<IF isReference THEN atom.style ELSE "keep">> \o rest.origins]

RECURSIVE Replay(_, _)
Replay(local, origins) ==
  IF Len(local) = 0 THEN <<>>
  ELSE <<IF Head(origins) = "keep" THEN Head(local) ELSE Reference(Head(origins))>>
       \o Replay(Tail(local), Tail(origins))

RECURSIVE Rebase(_, _)
Rebase(source, machine) ==
  IF Len(source) = 0 THEN <<>>
  ELSE LET atom == Head(source)
       IN <<IF atom.kind = "local-path"
            THEN IF atom.machine = "A" THEN Render(machine, atom.style) ELSE atom
            ELSE atom>> \o Rebase(Tail(source), machine)

VARIABLE source
Init == source \in Words(MaxLen)
Spec == Init /\ [][UNCHANGED source]_source

Wire == Encode(source)
OnB == Decode(Wire, "B")
PushedB == Replay(OnB.local, OnB.origins)
OnC == Decode(PushedB, "C")
PushedC == Replay(OnC.local, OnC.origins)

SameMachineKeepsExactSpelling == Decode(Wire, "A").local = source
DestinationRendersAvailableSpelling == OnB.local = Rebase(source, "B")
DestinationLiteralTextStaysLiteral == OnC.local = Rebase(source, "C")
RepushRetainsSourceSpelling == PushedB = Wire /\ PushedC = Wire
ReturnToSourceKeepsExactText == Decode(PushedC, "A").local = source

(***************************************************************************
Negative controls. The current codec keeps only raw/URI context. A repair
that stores origins but drops the spelling tag during replay is also lossy.
***************************************************************************)
CollapsedStyle(style) ==
  IF style \in {"binding", "canonical", "tilde-binding", "tilde-canonical"}
  THEN "binding" ELSE "uri-encoded-binding"

RECURSIVE Collapse(_)
Collapse(wire) ==
  IF Len(wire) = 0 THEN <<>>
  ELSE LET atom == Head(wire)
       IN <<IF atom.kind = "reference" THEN Reference(CollapsedStyle(atom.style)) ELSE atom>>
          \o Collapse(Tail(wire))

CanonicalSpellingIsLossless == Decode(Collapse(Wire), "A").local = source
DroppingSpellingOnRepushIsLossless == Decode(Collapse(PushedB), "A").local = source

====
