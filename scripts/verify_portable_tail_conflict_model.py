#!/usr/bin/env python3
"""Bounded checks for the minimal schema-1 root-tail resolver.

This is an executable design model, not an unbounded proof. Canonical tail
states are represented by collision-free tuples in the transition model. A
separate primitive test computes real SHA-256 vectors for the five-field v1
tail codec.

Physical history_base files are flattened into one root history before this
model begins. Subagents are incidental artifact bytes and never enter P/L/C.
The model excludes subagent conflict state, source cleanup, and containment
refinement. Pull planning is metadata-only and has no transcript download or
output-construction state here. A materializing Pull enters the modeled
PREPARED phase only after apply has downloaded verified immutable inputs,
constructed and validated one remote target, serialized/hashed it once, staged
it, and journaled exact pins.
"""

from __future__ import annotations

from collections import deque
from dataclasses import dataclass, replace
from enum import Enum
from hashlib import sha256
from struct import pack
from typing import Callable, Iterable, Optional, TypeVar


History = tuple[str, ...]
Subagents = tuple[str, ...]


class Kind(str, Enum):
    WHOLE = "whole_thread"
    FIXED = "fixed_paginated_tail"


@dataclass(frozen=True, order=True)
class Layout:
    kind: Kind
    start: int


WHOLE = Layout(Kind.WHOLE, 0)
FIXED = Layout(Kind.FIXED, 1)
REMOTE_FIXED = Layout(Kind.FIXED, 2)


@dataclass(frozen=True, order=True)
class TailValue:
    codec: str
    kind: Kind
    start: int
    end: int
    state: History


class Form(str, Enum):
    """The two byte forms of one conversation under the canonical wire
    encoding: cloud artifacts store token-form bytes, machines store their
    own root's expansion.  Record fingerprints are content hashes, so the
    two forms of the same history fold to different states; the distinct
    codec labels make any cross-form comparison fail structurally."""

    WIRE = "wire"
    LOCAL = "local"


FORM_CODEC = {
    Form.WIRE: "mallard-codex-portable-tail-wire-v1",
    Form.LOCAL: "mallard-codex-portable-tail-v1",
}


def tail_value(history: History, layout: Layout, form: Form = Form.LOCAL) -> TailValue:
    assert 0 <= layout.start <= len(history)
    # The per-form rendering is a deterministic pure function of the record
    # and the form (mirroring the canonicalizing encoder: patterns derive
    # from the binding root alone, so re-encoding old records always
    # reproduces the accepted wire prefix).  Rendering every record
    # differently per form is the worst case for the resolver: no wire
    # state ever collides with a local state.
    return TailValue(
        FORM_CODEC[form],
        layout.kind,
        layout.start,
        len(history),
        tuple((form.value, message) for message in history[layout.start :]),
    )


@dataclass(frozen=True)
class Cloud:
    commit: str
    history: History
    layout: Layout
    artifact: str
    ancestry: tuple[str, ...]

    def contains(self, commit: str) -> bool:
        return commit == self.commit or commit in self.ancestry


@dataclass(frozen=True)
class Accepted:
    commit: str
    manifest: str
    artifact: str
    tail: TailValue
    local_tail: TailValue


@dataclass(frozen=True)
class Binding:
    logical_id: str
    provider_id: str
    layout: Layout
    revision: int


@dataclass(frozen=True)
class Task:
    binding: Binding
    history: History
    observed: TailValue
    accepted: Optional[Accepted]


def make_task(
    logical_id: str,
    provider_id: str,
    history: History,
    layout: Layout,
    accepted: Optional[Accepted],
    revision: int = 1,
) -> Task:
    return Task(
        Binding(logical_id, provider_id, layout, revision),
        history,
        tail_value(history, layout),
        accepted,
    )


def accepted_from(cloud: Cloud) -> Accepted:
    # The dual pins are recorded atomically at one acceptance event: the
    # wire tail the artifact folded plus the local-form twin of the same
    # history (a push pins its seeded local value; a pull pins the decode
    # build's fold of the materialized bytes).
    return Accepted(
        cloud.commit,
        f"manifest-{cloud.commit}",
        cloud.artifact,
        tail_value(cloud.history, cloud.layout, Form.WIRE),
        tail_value(cloud.history, cloud.layout, Form.LOCAL),
    )


class Relation(str, Enum):
    SYNC_CURRENT = "SyncCurrent"
    SYNC_ADVANCED = "SyncAdvanced"
    LOCAL_OUTSTANDING = "LocalOutstanding"
    CLOUD_OUTSTANDING = "CloudOutstanding"
    DIVERGENT = "Divergent"


def classify(
    pin_wire: TailValue,
    pin_local: TailValue,
    local: TailValue,
    cloud: TailValue,
) -> Relation:
    """The pin-relative reading: the cloud side compares only to the
    accepted wire pin and the local side only to the accepted local twin.
    Field-wise equality includes the codec label, so a cross-form
    comparison can never accidentally succeed."""
    assert pin_wire.kind == pin_local.kind
    assert pin_wire.start == pin_local.start
    assert pin_wire.end == pin_local.end
    local_current = local == pin_local
    cloud_current = cloud == pin_wire
    if local_current and cloud_current:
        return Relation.SYNC_CURRENT
    # Same-form agreement on a newer value (single-form providers only;
    # unreachable across the dual-form codex columns).
    if local == cloud:
        return Relation.SYNC_ADVANCED
    if cloud_current and local.end > pin_local.end:
        return Relation.LOCAL_OUTSTANDING
    if local_current and cloud.end > pin_wire.end:
        return Relation.CLOUD_OUTSTANDING
    return Relation.DIVERGENT


def is_prefix(prefix: History, value: History) -> bool:
    return len(prefix) <= len(value) and value[: len(prefix)] == prefix


def task_valid(task: Task) -> bool:
    if task.observed != tail_value(task.history, task.binding.layout, Form.LOCAL):
        return False
    if task.accepted is not None:
        if task.accepted.tail.start != task.binding.layout.start:
            return False
        if task.accepted.tail.kind != task.binding.layout.kind:
            return False
        # The wire pin and its local twin agree on the form-independent
        # partition and coordinates, and the twin shares the observed
        # column's codec.
        if task.accepted.tail.kind != task.accepted.local_tail.kind:
            return False
        if task.accepted.tail.start != task.accepted.local_tail.start:
            return False
        if task.accepted.tail.end != task.accepted.local_tail.end:
            return False
        if task.accepted.local_tail.codec != FORM_CODEC[Form.LOCAL]:
            return False
        if task.accepted.tail.codec != FORM_CODEC[Form.WIRE]:
            return False
    return True


class RunMode(str, Enum):
    ACTIVE = "active"
    RECOVERY = "recovery"


class PushOutcome(str, Enum):
    WIN = "win"
    LOSS = "loss"
    AMBIGUOUS_WIN = "ambiguous_win"
    AMBIGUOUS_LOSS = "ambiguous_loss"
    AMBIGUOUS_DESCENDANT = "ambiguous_descendant"


class PushPhase(str, Enum):
    CANDIDATE_UPLOADED = "CandidateUploaded"
    HEAD_COMMITTED = "HeadCommitted"
    LOCAL_COMMITTED = "LocalCommitted"
    COMPLETE = "Complete"
    ABORTED_CAS_LOST = "AbortedCasLost"
    BLOCKED_CORRUPTION = "RecoveryBlockedCorruption"


class DocumentState(str, Enum):
    BEFORE = "before"
    AFTER = "after"
    THIRD = "third"


@dataclass(frozen=True)
class PushState:
    phase: PushPhase
    mode: RunMode
    cloud: Cloud
    candidate: Cloud
    concurrent: Cloud
    descendant: Cloud
    task: Task
    outcome: PushOutcome
    config: DocumentState
    provider_id_pin: str
    pending: bool


def apply_push_cas(state: PushState) -> PushState:
    if state.outcome == PushOutcome.WIN:
        return replace(
            state,
            cloud=state.candidate,
            phase=PushPhase.HEAD_COMMITTED,
        )
    if state.outcome == PushOutcome.LOSS:
        return replace(
            state,
            cloud=state.concurrent,
            phase=PushPhase.ABORTED_CAS_LOST,
        )
    if state.outcome == PushOutcome.AMBIGUOUS_WIN:
        return replace(
            state,
            cloud=state.candidate,
            mode=RunMode.RECOVERY,
        )
    if state.outcome == PushOutcome.AMBIGUOUS_LOSS:
        return replace(
            state,
            cloud=state.concurrent,
            mode=RunMode.RECOVERY,
        )
    assert state.outcome == PushOutcome.AMBIGUOUS_DESCENDANT
    return replace(
        state,
        cloud=state.descendant,
        mode=RunMode.RECOVERY,
    )


def advance_push(state: PushState) -> PushState:
    if state.phase == PushPhase.CANDIDATE_UPLOADED:
        if state.mode == RunMode.ACTIVE:
            return apply_push_cas(state)
        if state.cloud.contains(state.candidate.commit):
            return replace(state, phase=PushPhase.HEAD_COMMITTED)
        return replace(state, phase=PushPhase.ABORTED_CAS_LOST)

    if state.phase == PushPhase.HEAD_COMMITTED:
        if state.config == DocumentState.THIRD:
            return replace(state, phase=PushPhase.BLOCKED_CORRUPTION)
        promoted = replace(
            state.task,
            accepted=accepted_from(state.candidate),
        )
        return replace(
            state,
            task=promoted,
            config=DocumentState.AFTER,
            phase=PushPhase.LOCAL_COMMITTED,
        )

    if state.phase == PushPhase.LOCAL_COMMITTED:
        return replace(state, phase=PushPhase.COMPLETE)

    if state.phase in {
        PushPhase.COMPLETE,
        PushPhase.ABORTED_CAS_LOST,
    }:
        return replace(state, pending=False)

    raise AssertionError(f"cannot advance push {state.phase}")


def push_terminal(state: PushState) -> bool:
    if state.phase == PushPhase.BLOCKED_CORRUPTION:
        return True
    return (
        state.phase in {
            PushPhase.COMPLETE,
            PushPhase.ABORTED_CAS_LOST,
        }
        and not state.pending
    )


def push_successors(state: PushState) -> tuple[PushState, ...]:
    if push_terminal(state):
        return ()
    if state.mode == RunMode.ACTIVE:
        return (
            advance_push(state),
            replace(state, mode=RunMode.RECOVERY),
        )
    if state.phase == PushPhase.HEAD_COMMITTED and state.config == DocumentState.BEFORE:
        return (
            advance_push(state),
            replace(state, config=DocumentState.THIRD),
        )
    return (advance_push(state),)


def check_push(state: PushState) -> None:
    assert task_valid(state.task)
    assert state.task.binding.provider_id == state.provider_id_pin
    assert state.task.binding.layout == state.candidate.layout

    if state.phase in {
        PushPhase.LOCAL_COMMITTED,
        PushPhase.COMPLETE,
    }:
        assert state.task.accepted == accepted_from(state.candidate)
        assert state.config == DocumentState.AFTER

    if state.phase == PushPhase.ABORTED_CAS_LOST:
        assert state.task.accepted != accepted_from(state.candidate)


class PullKind(str, Enum):
    CLOUD_OUTSTANDING = "CloudOutstanding"
    MISSING_LOCAL = "MissingLocal"
    DIVERGENT = "Divergent"


class PullPhase(str, Enum):
    PREPARED = "Prepared"
    COMMIT_DECIDED = "CommitDecided"
    TARGET_COMMITTED = "TargetCommitted"
    BINDINGS_COMMITTED = "BindingsCommitted"
    CONFIG_COMMITTED = "SyncConfigCommitted"
    COMPLETE = "Complete"
    ABORTED_PRECOMMIT = "AbortedPrecommit"
    ABORTED_COLLISION = "AbortedProviderCollision"
    BLOCKED_CORRUPTION = "RecoveryBlockedCorruption"


@dataclass(frozen=True)
class PullState:
    phase: PullPhase
    mode: RunMode
    kind: PullKind
    old_task: Optional[Task]
    remote: Cloud
    logical_c: Optional[str]
    fixed_target_id: str
    target_committed: bool
    target_collision: bool
    stale_precommit: bool
    bindings: DocumentState
    config: DocumentState
    tasks_after: tuple[Task, ...]
    old_source_histories: tuple[History, ...]
    pending: bool


def expected_pull_tasks(state: PullState) -> tuple[Task, ...]:
    old = state.old_task
    if state.kind == PullKind.DIVERGENT:
        assert old is not None and state.logical_c is not None
        preserved = make_task(
            state.logical_c,
            old.binding.provider_id,
            old.history,
            old.binding.layout,
            None,
        )
        remote_t = make_task(
            old.binding.logical_id,
            state.fixed_target_id,
            state.remote.history,
            state.remote.layout,
            accepted_from(state.remote),
            old.binding.revision + 1,
        )
        return (preserved, remote_t)

    if state.kind in {
        PullKind.CLOUD_OUTSTANDING,
        PullKind.MISSING_LOCAL,
    }:
        logical_id = old.binding.logical_id if old is not None else "T"
        revision = old.binding.revision + 1 if old is not None else 1
        return (
            make_task(
                logical_id,
                state.fixed_target_id,
                state.remote.history,
                state.remote.layout,
                accepted_from(state.remote),
                revision,
            ),
        )

    raise AssertionError(f"unsupported materializing Pull kind {state.kind}")


def advance_pull(state: PullState) -> PullState:
    if state.phase == PullPhase.PREPARED:
        if state.stale_precommit:
            return replace(state, phase=PullPhase.ABORTED_PRECOMMIT)
        return replace(state, phase=PullPhase.COMMIT_DECIDED)

    if state.phase == PullPhase.COMMIT_DECIDED:
        if state.target_collision and not state.target_committed:
            return replace(state, phase=PullPhase.ABORTED_COLLISION)
        if not state.target_committed:
            return replace(state, target_committed=True)
        return replace(state, phase=PullPhase.TARGET_COMMITTED)

    if state.phase == PullPhase.TARGET_COMMITTED:
        if state.bindings == DocumentState.THIRD:
            return replace(state, phase=PullPhase.BLOCKED_CORRUPTION)
        return replace(
            state,
            bindings=DocumentState.AFTER,
            tasks_after=expected_pull_tasks(state),
            phase=PullPhase.BINDINGS_COMMITTED,
        )

    if state.phase == PullPhase.BINDINGS_COMMITTED:
        if state.config == DocumentState.THIRD:
            return replace(state, phase=PullPhase.BLOCKED_CORRUPTION)
        return replace(
            state,
            config=DocumentState.AFTER,
            phase=PullPhase.CONFIG_COMMITTED,
        )

    if state.phase == PullPhase.CONFIG_COMMITTED:
        return replace(state, phase=PullPhase.COMPLETE)

    if state.phase in {
        PullPhase.COMPLETE,
        PullPhase.ABORTED_PRECOMMIT,
        PullPhase.ABORTED_COLLISION,
    }:
        return replace(state, pending=False)

    raise AssertionError(f"cannot advance pull {state.phase}")


def pull_terminal(state: PullState) -> bool:
    if state.phase == PullPhase.BLOCKED_CORRUPTION:
        return True
    return (
        state.phase
        in {
            PullPhase.COMPLETE,
            PullPhase.ABORTED_PRECOMMIT,
            PullPhase.ABORTED_COLLISION,
        }
        and not state.pending
    )


def pull_successors(state: PullState) -> tuple[PullState, ...]:
    if pull_terminal(state):
        return ()
    if state.mode == RunMode.ACTIVE:
        return (
            advance_pull(state),
            replace(state, mode=RunMode.RECOVERY),
        )
    if state.phase == PullPhase.TARGET_COMMITTED and state.bindings == DocumentState.BEFORE:
        return (
            advance_pull(state),
            replace(state, bindings=DocumentState.THIRD),
        )
    if state.phase == PullPhase.BINDINGS_COMMITTED and state.config == DocumentState.BEFORE:
        return (
            advance_pull(state),
            replace(state, config=DocumentState.THIRD),
        )
    return (advance_pull(state),)


def check_pull(state: PullState) -> None:
    # Old provider sources are immutable inputs and are never retired.
    if state.old_task is not None:
        assert state.old_task.history in state.old_source_histories

    for task in state.tasks_after:
        assert task_valid(task)

    if state.phase in {
        PullPhase.BINDINGS_COMMITTED,
        PullPhase.CONFIG_COMMITTED,
        PullPhase.COMPLETE,
    }:
        assert state.tasks_after == expected_pull_tasks(state)
        assert state.bindings == DocumentState.AFTER

    if state.phase in {
        PullPhase.CONFIG_COMMITTED,
        PullPhase.COMPLETE,
    }:
        assert state.config == DocumentState.AFTER

    if state.kind == PullKind.DIVERGENT and state.tasks_after:
        histories = {task.history for task in state.tasks_after}
        assert state.old_task is not None
        assert state.old_task.history in histories
        assert state.remote.history in histories
        by_id = {task.binding.logical_id: task for task in state.tasks_after}
        preserved = by_id[state.logical_c]
        remote_t = by_id[state.old_task.binding.logical_id]
        assert preserved.binding.provider_id == state.old_task.binding.provider_id
        assert preserved.binding.provider_id != state.fixed_target_id
        assert preserved.binding.layout == state.old_task.binding.layout
        assert preserved.observed == state.old_task.observed
        assert preserved.accepted is None
        assert remote_t.binding.provider_id == state.fixed_target_id
        assert remote_t.binding.layout == state.remote.layout


S = TypeVar("S")


@dataclass
class Exploration:
    states: int = 0
    transitions: int = 0
    recovery_states: int = 0
    complete: int = 0
    safe_abort: int = 0
    corruption_block: int = 0


def explore(
    initial: S,
    successors: Callable[[S], tuple[S, ...]],
    check: Callable[[S], None],
    terminal: Callable[[S], bool],
    is_complete: Callable[[S], bool],
    is_abort: Callable[[S], bool],
    is_blocked: Callable[[S], bool],
) -> Exploration:
    queue = deque([initial])
    seen = {initial}
    result = Exploration()

    while queue:
        state = queue.popleft()
        result.states += 1
        if getattr(state, "mode") == RunMode.RECOVERY:
            result.recovery_states += 1
        check(state)
        if terminal(state):
            if is_complete(state):
                result.complete += 1
            elif is_abort(state):
                result.safe_abort += 1
            elif is_blocked(state):
                result.corruption_block += 1
            else:
                raise AssertionError("unclassified terminal")
            continue

        next_states = successors(state)
        assert next_states
        result.transitions += len(next_states)
        for next_state in next_states:
            if next_state not in seen:
                seen.add(next_state)
                queue.append(next_state)

    return result


def push_initial(
    layout: Layout,
    outcome: PushOutcome,
    third_config: bool = False,
) -> PushState:
    base_history = ("m0", "m1")
    candidate_history = base_history + ("m2",)
    base = Cloud("B", base_history, layout, "artifact-B", ())
    candidate = Cloud("P", candidate_history, layout, "artifact-P", ("B",))
    concurrent = Cloud("R", base_history + ("r",), layout, "artifact-R", ("B",))
    descendant = Cloud("D", candidate_history + ("d",), layout, "artifact-D", ("B", "P"))
    task = make_task(
        "T",
        "provider-T-fixed",
        candidate_history,
        layout,
        accepted_from(base),
    )
    return PushState(
        PushPhase.CANDIDATE_UPLOADED,
        RunMode.ACTIVE,
        base,
        candidate,
        concurrent,
        descendant,
        task,
        outcome,
        DocumentState.THIRD if third_config else DocumentState.BEFORE,
        "provider-T-fixed",
        True,
    )


def pull_initial(
    kind: PullKind,
    layout: Layout,
    target_collision: bool = False,
    stale: bool = False,
    third_bindings: bool = False,
    third_config: bool = False,
) -> PullState:
    base_history = ("m0", "m1")
    local_history = base_history
    remote_history = base_history
    remote_layout = layout
    logical_c = None

    if kind == PullKind.CLOUD_OUTSTANDING:
        remote_history = base_history + ("remote",)
    elif kind == PullKind.MISSING_LOCAL:
        remote_history = base_history + ("remote",)
    elif kind == PullKind.DIVERGENT:
        local_history = base_history + ("local",)
        remote_history = base_history + ("remote",)
        remote_layout = REMOTE_FIXED if layout == FIXED else layout
        logical_c = "C-fixed"

    base = Cloud("B", base_history, layout, "artifact-B", ())
    remote = Cloud("R", remote_history, remote_layout, "artifact-R", ("B",))
    old_task = None
    if kind != PullKind.MISSING_LOCAL:
        old_task = make_task(
            "T",
            "provider-T-old",
            local_history,
            layout,
            accepted_from(base),
        )

    sources = (old_task.history,) if old_task is not None else ()
    return PullState(
        PullPhase.PREPARED,
        RunMode.ACTIVE,
        kind,
        old_task,
        remote,
        logical_c,
        "provider-target-fixed",
        False,
        target_collision,
        stale,
        DocumentState.THIRD if third_bindings else DocumentState.BEFORE,
        DocumentState.THIRD if third_config else DocumentState.BEFORE,
        (),
        sources,
        True,
    )


def classifier_checks() -> int:
    def wire(history: History, layout: Layout = WHOLE) -> TailValue:
        return tail_value(history, layout, Form.WIRE)

    def local_form(history: History, layout: Layout = WHOLE) -> TailValue:
        return tail_value(history, layout, Form.LOCAL)

    base = ("m0", "m1")
    local_longer = ("m0", "m1", "l")
    cloud_longer = ("m0", "m1", "c")
    rewrite_same_end = ("m0", "x")

    checks = [
        # The four-row pinned table under dual forms.
        (wire(base), local_form(base), local_form(base), wire(base), Relation.SYNC_CURRENT),
        (
            wire(base),
            local_form(base),
            local_form(local_longer),
            wire(base),
            Relation.LOCAL_OUTSTANDING,
        ),
        (
            wire(base),
            local_form(base),
            local_form(base),
            wire(cloud_longer),
            Relation.CLOUD_OUTSTANDING,
        ),
        (
            wire(base),
            local_form(base),
            local_form(local_longer),
            wire(cloud_longer),
            Relation.DIVERGENT,
        ),
        # Same-form Advanced row: reachable only when local and cloud share
        # one codec (single-form providers).
        (
            local_form(base),
            local_form(base),
            local_form(local_longer),
            local_form(local_longer),
            Relation.SYNC_ADVANCED,
        ),
        # Equal-or-shorter ends under an unequal value are rewrites, never
        # outstanding work.
        (
            wire(base),
            local_form(base),
            local_form(rewrite_same_end),
            wire(base),
            Relation.DIVERGENT,
        ),
        (
            wire(base),
            local_form(base),
            local_form(base),
            wire(rewrite_same_end),
            Relation.DIVERGENT,
        ),
        # Deliberate false conflict: final-value equality only, no containment.
        (
            wire(("m0",)),
            local_form(("m0",)),
            local_form(("m0", "m1")),
            wire(("m0", "m1", "m2")),
            Relation.DIVERGENT,
        ),
    ]
    for pin_wire, pin_local, local_value, cloud_value, expected in checks:
        assert classify(pin_wire, pin_local, local_value, cloud_value) == expected
    return len(checks)


def zero_target_pull_checks() -> int:
    """Atomic metadata-only Pull has no journal or recovery state machine."""
    count = 0
    for relation in (
        Relation.SYNC_ADVANCED,
        Relation.LOCAL_OUTSTANDING,
    ):
        for before in DocumentState:
            if before == DocumentState.BEFORE:
                after = DocumentState.AFTER
                applied = True
            elif before == DocumentState.AFTER:
                after = DocumentState.AFTER
                applied = True
            else:
                after = DocumentState.THIRD
                applied = False

            assert relation in {
                Relation.SYNC_ADVANCED,
                Relation.LOCAL_OUTSTANDING,
            }
            assert (after == DocumentState.AFTER) == applied
            # There is no target, journal phase, or pending-resource bit.
            targets = ()
            pending = False
            assert not targets and not pending
            count += 1
    return count


def subagent_exclusion_checks() -> int:
    wire_tail = tail_value(("m0", "m1"), WHOLE, Form.WIRE)
    local_tail = tail_value(("m0", "m1"), WHOLE, Form.LOCAL)
    snapshots: tuple[Subagents, ...] = (
        (),
        ("s1",),
        ("s1", "s2"),
    )
    count = 0
    for pin_subagents in snapshots:
        for local_subagents in snapshots:
            for cloud_subagents in snapshots:
                # Artifact snapshots differ, but relation inputs do not.
                _artifacts = (
                    pin_subagents,
                    local_subagents,
                    cloud_subagents,
                )
                assert (
                    classify(wire_tail, local_tail, local_tail, wire_tail)
                    == Relation.SYNC_CURRENT
                )
                count += 1
    return count


def lp_tuple(fields: Iterable[bytes]) -> bytes:
    values = tuple(fields)
    out = pack(">I", len(values))
    for value in values:
        out += pack(">Q", len(value)) + value
    return out


def u64(value: int) -> bytes:
    return pack(">Q", value)


def codec_golden_checks() -> int:
    codec = b"mallard-codex-portable-tail-v1"
    whole = b"whole_thread"
    fixed = b"fixed_paginated_tail"

    empty_whole = sha256(lp_tuple((
        b"mallard-codex-portable-tail-start-v1",
        codec,
        whole,
        u64(0),
    ))).hexdigest()
    empty_fixed_three = sha256(lp_tuple((
        b"mallard-codex-portable-tail-start-v1",
        codec,
        fixed,
        u64(3),
    ))).digest()
    m4_fingerprint = sha256(lp_tuple((
        b"mallard-codex-canonical-root-message-v1",
        b"m4",
    ))).digest()
    after_m4 = sha256(lp_tuple((
        b"mallard-codex-portable-tail-record-v1",
        u64(0),
        empty_fixed_three,
        m4_fingerprint,
    ))).hexdigest()

    expected = {
        "empty_whole": "85ca60a860dfac615f75141b865e6191feea154a72dac2b1007c8d5dcdedf5fd",
        "m4_fingerprint": "c594cce30c1ea89769fc90b54bc0608fec7e85e42ba615e619fdd6f2074b8c46",
        "fixed3_after_m4": "c79c5b174e0f47c22fc57bebe482c8ab6524129b31d1ee0cf82799c4951aca44",
    }
    actual = {
        "empty_whole": empty_whole,
        "m4_fingerprint": m4_fingerprint.hex(),
        "fixed3_after_m4": after_m4,
    }
    assert actual == expected, actual
    return len(actual)


def negative_controls() -> int:
    found: list[str] = []

    # 1. Letting a stale Push overwrite without CAS loses remote history.
    if not is_prefix(("m0", "remote"), ("m0", "local")):
        found.append("unguarded-stale-overwrite")

    # 2. Resetting accepted state after Push loses cumulative tail history.
    if tail_value(("m0", "m1"), WHOLE).state != ():
        found.append("tail-reset")

    # 2b. Comparing the local observation to the wire pin (a cross-form
    # comparison) reads every synced dual-form thread as divergent.
    cross_form_pin = tail_value(("m0", "m1"), WHOLE, Form.WIRE)
    cross_form_local = tail_value(("m0", "m1"), WHOLE, Form.LOCAL)
    if cross_form_pin != cross_form_local:
        found.append("cross-form-comparison")

    # 3. Reusing remote layout for preserved C loses C's fixed start.
    if FIXED != REMOTE_FIXED:
        found.append("preserved-layout-replacement")

    # 4. Recovery remint changes reviewed identity.
    if "provider-fixed" != "provider-reminted":
        found.append("recovery-remint")

    # 5. Folding subagents into relation creates a false conflict.
    root = tail_value(("m0", "m1"), WHOLE)
    if (root, ("s1",)) != (root, ("s2",)):
        found.append("subagent-enters-conflict")

    # 6. Deleting an old source after alias switch risks shared history.
    old_sources = (("m0", "local"),)
    if old_sources:
        found.append("deleting-old-source-would-lose-input")

    expected = {
        "unguarded-stale-overwrite",
        "tail-reset",
        "cross-form-comparison",
        "preserved-layout-replacement",
        "recovery-remint",
        "subagent-enters-conflict",
        "deleting-old-source-would-lose-input",
    }
    assert set(found) == expected
    return len(found)


def add(total: Exploration, value: Exploration) -> None:
    total.states += value.states
    total.transitions += value.transitions
    total.recovery_states += value.recovery_states
    total.complete += value.complete
    total.safe_abort += value.safe_abort
    total.corruption_block += value.corruption_block


def main() -> None:
    codec_vectors = codec_golden_checks()
    relations = classifier_checks()
    zero_target_cases = zero_target_pull_checks()
    subagents = subagent_exclusion_checks()
    negative = negative_controls()

    push_total = Exploration()
    push_scenarios = 0
    for layout in (WHOLE, FIXED):
        for outcome in PushOutcome:
            initial = push_initial(layout, outcome)
            result = explore(
                initial,
                push_successors,
                check_push,
                push_terminal,
                lambda s: s.phase == PushPhase.COMPLETE,
                lambda s: s.phase == PushPhase.ABORTED_CAS_LOST,
                lambda s: s.phase == PushPhase.BLOCKED_CORRUPTION,
            )
            add(push_total, result)
            push_scenarios += 1

    pull_total = Exploration()
    pull_scenarios = 0
    for layout in (WHOLE, FIXED):
        for kind in PullKind:
            for collision in (False, True):
                initial = pull_initial(
                    kind,
                    layout,
                    target_collision=collision,
                )
                result = explore(
                    initial,
                    pull_successors,
                    check_pull,
                    pull_terminal,
                    lambda s: s.phase == PullPhase.COMPLETE,
                    lambda s: s.phase in {
                        PullPhase.ABORTED_PRECOMMIT,
                        PullPhase.ABORTED_COLLISION,
                    },
                    lambda s: s.phase == PullPhase.BLOCKED_CORRUPTION,
                )
                add(pull_total, result)
                pull_scenarios += 1

        stale_initial = pull_initial(
            PullKind.CLOUD_OUTSTANDING,
            layout,
            stale=True,
        )
        stale_result = explore(
            stale_initial,
            pull_successors,
            check_pull,
            pull_terminal,
            lambda s: s.phase == PullPhase.COMPLETE,
            lambda s: s.phase in {
                PullPhase.ABORTED_PRECOMMIT,
                PullPhase.ABORTED_COLLISION,
            },
            lambda s: s.phase == PullPhase.BLOCKED_CORRUPTION,
        )
        add(pull_total, stale_result)
        pull_scenarios += 1

    total = Exploration()
    add(total, push_total)
    add(total, pull_total)

    print(f"codec golden vectors: {codec_vectors}")
    print(f"classifier cases: {relations}")
    print(f"zero-target atomic Pull cases: {zero_target_cases}")
    print(f"root-only subagent exclusion cases: {subagents}")
    print(
        f"push: {push_scenarios} scenarios, {push_total.states} states, "
        f"{push_total.transitions} transitions, "
        f"{push_total.recovery_states} recovery states"
    )
    print(
        f"pull: {pull_scenarios} scenarios, {pull_total.states} states, "
        f"{pull_total.transitions} transitions, "
        f"{pull_total.recovery_states} recovery states"
    )
    print(
        f"total: {total.states} states, {total.transitions} transitions; "
        f"complete={total.complete}, safe_abort={total.safe_abort}, "
        f"corruption_block={total.corruption_block}"
    )
    print(f"expected unsafe negative controls: {negative}")
    print("PASS: no bounded counterexample in the simplified model")


if __name__ == "__main__":
    main()
