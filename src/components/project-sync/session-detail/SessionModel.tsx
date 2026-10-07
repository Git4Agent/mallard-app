import type { SessionMessageSettings, SessionMessageSettingsSource, SessionTimelineEvent } from "../../../types";
import { PathBarPopover } from "../../sync-review/SyncPathBar";

const SOURCE_LABEL: Record<SessionMessageSettingsSource, string> = {
  turn: "Turn settings",
  reply: "Reply record",
  linked: "Linked replies",
  none: "Not recorded",
};

/** `model · level`, each field as recorded; Unknown where the transcript
 * recorded none, and the one word when it recorded neither. */
export function sessionModelLabel(settings: SessionMessageSettings): string {
  if (settings.model == null && settings.effort == null) return "Unknown";
  return `${settings.model ?? "Unknown"} · ${settings.effort ?? "Unknown"}`;
}

/** The messages a change of model or thinking level applies from, with what
 * applies: the walk starts from the session's start settings, and a message
 * that recorded nothing says nothing. */
export function sessionModelNotes(
  events: readonly SessionTimelineEvent[],
  start: SessionMessageSettings | null | undefined,
): Map<string, SessionMessageSettings> {
  const notes = new Map<string, SessionMessageSettings>();
  let model = start?.model ?? null;
  let effort = start?.effort ?? null;
  for (const event of events) {
    const next = event.settings;
    if ((event.kind !== "prompt" && event.kind !== "response") || !next) continue;
    if (next.model == null && next.effort == null) continue;
    if ((next.model ?? null) === model && (next.effort ?? null) === effort) continue;
    notes.set(event.event_id, next);
    model = next.model ?? null;
    effort = next.effort ?? null;
  }
  return notes;
}

/** The facts behind a chip: Model, Thinking, Source. */
function SessionModelFacts({ settings }: { settings: SessionMessageSettings }) {
  return (
    <dl className="session-detail-model-facts">
      <div><dt>Model</dt><dd>{settings.model != null ? <code>{settings.model}</code> : "Unknown"}</dd></div>
      <div><dt>Thinking</dt><dd>{settings.effort ?? "Unknown"}</dd></div>
      <div><dt>Source</dt><dd>{SOURCE_LABEL[settings.source]}</dd></div>
    </dl>
  );
}

/**
 * The fact chip: what the session started with, `model · level`, among the
 * header's facts. It opens the facts in the path bar's popover box, as the
 * notes chip does.
 */
export function SessionModelChip({ settings }: { settings: SessionMessageSettings }) {
  const label = sessionModelLabel(settings);
  return (
    <PathBarPopover
      id="session-model"
      label="Model and thinking"
      className="session-detail-chip session-detail-model-chip"
      ariaLabel={`Model and thinking: ${label}`}
      face={settings.model != null ? <><code>{settings.model}</code> · {settings.effort ?? "Unknown"}</> : label}
    >
      {() => <SessionModelFacts settings={settings} />}
    </PathBarPopover>
  );
}

/**
 * A change of model or thinking level: a note on the rail before the message
 * it applies from, naming what applies from here. The session's starting
 * settings are the header's chip; the transcript marks only the changes.
 */
export function SessionModelNote({ eventId, settings }: { eventId: string; settings: SessionMessageSettings }) {
  const label = sessionModelLabel(settings);
  return (
    <div className="session-detail-model-note">
      <PathBarPopover
        id={`session-model:${eventId}`}
        label="Model and thinking from here"
        className="session-detail-model-note-chip"
        ariaLabel={`Model and thinking from here: ${label}`}
        face={label}
      >
        {() => <SessionModelFacts settings={settings} />}
      </PathBarPopover>
    </div>
  );
}
