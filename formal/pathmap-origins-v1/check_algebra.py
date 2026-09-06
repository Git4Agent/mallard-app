#!/usr/bin/env python3
"""Check the proposed typed path algebra with Z3, without provider files.

Literal data is an arbitrary finite sequence of bytes. Lists have no length
bound. Constructor cases are checked with explicit induction hypotheses on
the tail. This checks the algebra after lexical decomposition; it does not
verify a concrete escape parser, UTF-8 offsets, JSON parser, or Rust code.
"""

import json

import z3 as z


Bytes = z.SeqSort(z.BitVecSort(8))
Style, STYLES = z.EnumSort("Style", [
    "binding", "canonical", "tilde_binding", "tilde_canonical",
    "uri_raw_binding", "uri_raw_canonical",
    "uri_encoded_binding", "uri_encoded_canonical",
])
Machine, (A, B, C) = z.EnumSort("Machine", ["A", "B", "C"])

# All data is inside the Literal constructor. Marker-like bytes cannot
# become a Reference merely because of their contents.
Wire = z.Datatype("Wire")
Wire.declare("End")
Wire.declare("Literal", ("literal_bytes", Bytes), ("literal_tail", Wire))
Wire.declare("Reference", ("reference_style", Style), ("reference_tail", Wire))
Wire = Wire.create()

Origin = z.Datatype("Origin")
Origin.declare("Keep")
Origin.declare("Mapped", ("mapped_style", Style))
Origin = Origin.create()

Local = z.Datatype("Local")
Local.declare("Done")
Local.declare("Chunk", ("chunk_bytes", Bytes), ("chunk_origin", Origin), ("chunk_tail", Local))
Local = Local.create()

# Render is deliberately unconstrained. It can return any byte sequence,
# and two different styles can render to equal text on a destination.
render = z.Function("render", Machine, Style, Bytes)
w = z.Const("w", Wire)
local = z.Const("local", Local)
machine = z.Const("machine", Machine)

decode = z.RecFunction("decode", Wire, Machine, Local)
z.RecAddDefinition(decode, (w, machine), z.If(
    Wire.is_End(w), Local.Done,
    z.If(Wire.is_Literal(w),
         Local.Chunk(Wire.literal_bytes(w), Origin.Keep,
                     decode(Wire.literal_tail(w), machine)),
         Local.Chunk(render(machine, Wire.reference_style(w)),
                     Origin.Mapped(Wire.reference_style(w)),
                     decode(Wire.reference_tail(w), machine))),
))

replay = z.RecFunction("replay", Local, Wire)
z.RecAddDefinition(replay, (local,), z.If(
    Local.is_Done(local), Wire.End,
    z.If(Origin.is_Keep(Local.chunk_origin(local)),
         Wire.Literal(Local.chunk_bytes(local), replay(Local.chunk_tail(local))),
         Wire.Reference(Origin.mapped_style(Local.chunk_origin(local)),
                        replay(Local.chunk_tail(local)))),
))

flatten = z.RecFunction("flatten", Local, Bytes)
z.RecAddDefinition(flatten, (local,), z.If(
    Local.is_Done(local), z.Empty(Bytes),
    z.Concat(Local.chunk_bytes(local), flatten(Local.chunk_tail(local))),
))

# Source text after decomposition. A project's exact registered source
# spelling is render(sourceMachine, style). Literal bytes are copied.
source_text = z.RecFunction("source_text", Wire, Machine, Bytes)
z.RecAddDefinition(source_text, (w, machine), z.If(
    Wire.is_End(w), z.Empty(Bytes),
    z.If(Wire.is_Literal(w),
         z.Concat(Wire.literal_bytes(w), source_text(Wire.literal_tail(w), machine)),
         z.Concat(render(machine, Wire.reference_style(w)),
                  source_text(Wire.reference_tail(w), machine))),
))

results = []


def check(name, assumptions, claim, expected=z.unsat):
    solver = z.Solver()
    solver.set(timeout=15000)
    solver.add(*assumptions, z.Not(claim))
    outcome = solver.check()
    result = {"check": name, "expected": str(expected), "result": str(outcome)}
    results.append(result)
    print(json.dumps(result), flush=True)
    if outcome != expected:
        if outcome == z.unknown:
            print(solver.reason_unknown(), flush=True)
        raise SystemExit(1)


data = z.Const("arbitrary_literal_bytes", Bytes)
tail = z.Const("arbitrary_wire_tail", Wire)
style = z.Const("arbitrary_style", Style)
other_style = z.Const("other_style", Style)

# Structural induction on wire lists: base and both constructor cases.
text_ih = flatten(decode(tail, machine)) == source_text(tail, machine)
check("same-machine: empty", [],
      flatten(decode(Wire.End, machine)) == source_text(Wire.End, machine))
check("same-machine: arbitrary literal constructor", [text_ih],
      flatten(decode(Wire.Literal(data, tail), machine))
      == source_text(Wire.Literal(data, tail), machine))
check("same-machine: spelling constructor", [text_ih],
      flatten(decode(Wire.Reference(style, tail), machine))
      == source_text(Wire.Reference(style, tail), machine))

replay_ih = replay(decode(tail, machine)) == tail
check("replay: empty", [], replay(decode(Wire.End, machine)) == Wire.End)
check("replay: arbitrary literal constructor", [replay_ih],
      replay(decode(Wire.Literal(data, tail), machine)) == Wire.Literal(data, tail))
check("replay: spelling constructor", [replay_ih],
      replay(decode(Wire.Reference(style, tail), machine)) == Wire.Reference(style, tail))

# Consequence of the replay theorem. No particular machine spelling or
# literal content is assumed. The tail can contain any finite number of
# chunks. The assumptions cite the theorem just proved by constructor cases.
check("return A after B and C", [
    replay(decode(tail, B)) == tail,
    replay(decode(tail, C)) == tail,
    flatten(decode(tail, A)) == source_text(tail, A),
], flatten(decode(replay(decode(replay(decode(tail, B)), C)), A)) == source_text(tail, A))

check("equal destination text retains distinct spelling origins", [
    style != other_style,
    render(B, style) == render(B, other_style),
], decode(Wire.Reference(style, Wire.End), B)
   != decode(Wire.Reference(other_style, Wire.End), B))

check("literal root text differs from a path reference", [data == render(B, style)],
      decode(Wire.Literal(data, Wire.End), B)
      != decode(Wire.Reference(style, Wire.End), B))

# Negative controls must produce counterexamples, not solver failures.
check("negative: discard original spelling", [
    render(A, STYLES[0]) != render(A, STYLES[2]),
], flatten(decode(Wire.Reference(STYLES[0], Wire.End), A))
   == source_text(Wire.Reference(STYLES[2], Wire.End), A), z.sat)

blind_replay = z.If(data == render(B, STYLES[0]),
                   Wire.Reference(STYLES[0], Wire.End), Wire.Literal(data, Wire.End))
check("negative: infer path origin from destination text", [data == render(B, STYLES[0])],
      blind_replay == Wire.Literal(data, Wire.End), z.sat)

check("negative: alter one literal byte", [],
      z.Concat(data, z.Unit(z.BitVecVal(0, 8))) == data, z.sat)

# The hash is an arbitrary deterministic function. This makes no claim
# about SHA-256 or JSON serialization. A fresh record is immutable between
# capture and replay, so its old hash check is H(record) == H(record).
Record = z.DeclareSort("Record")
record_hash = z.Function("record_hash", Record, z.BitVecSort(256))
captured_record = z.Const("captured_record", Record)
checked_record = z.Const("checked_record", Record)
inverse_replay_ok = z.Bool("inverse_replay_ok")
old_gate = z.And(record_hash(checked_record) == record_hash(captured_record),
                 inverse_replay_ok)
new_fresh_gate = inverse_replay_ok
check("fresh unchanged record: omit reflexive hash check", [
    checked_record == captured_record,
], old_gate == new_fresh_gate)

# Without the freshness condition, bypassing the stored hash is unsound.
check("negative: omit stored-origin hash check", [
    record_hash(checked_record) != record_hash(captured_record),
    inverse_replay_ok,
], old_gate == new_fresh_gate, z.sat)

print(json.dumps({"z3_version": z.get_version_string(), "checks": len(results),
                  "positive": sum(item["expected"] == "unsat" for item in results),
                  "negative": sum(item["expected"] == "sat" for item in results)}), flush=True)
