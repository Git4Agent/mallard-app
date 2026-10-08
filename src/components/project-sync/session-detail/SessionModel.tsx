import type { SessionMessageSettings, SessionMessageSettingsSource, SessionTimelineEvent } from "../../../types";
import { PathBarPopover } from "../../sync-review/SyncPathBar";
import { t as translate, useTranslation } from "../../../i18n";

function sourceLabel(source: SessionMessageSettingsSource): string {
  switch (source) {
    case "turn": return translate("session:model.sourceTurn");
    case "reply": return translate("session:model.sourceReply");
    case "linked": return translate("session:model.sourceLinked");
    case "none": return translate("session:model.sourceNone");
  }
}

/** `model · level`, each field as recorded; Unknown where the transcript
 * recorded none, and the one word when it recorded neither. */
export function sessionModelLabel(settings: SessionMessageSettings): string {
  const unknown = translate("session:model.unknown");
  if (settings.model == null && settings.effort == null) return unknown;
  return `${settings.model ?? unknown} · ${settings.effort ?? unknown}`;
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
  const { t } = useTranslation("session");
  return (
    <dl className="session-detail-model-facts">
      <div><dt>{t("model.model")}</dt><dd>{settings.model != null ? <code>{settings.model}</code> : t("model.unknown")}</dd></div>
      <div><dt>{t("model.thinking")}</dt><dd>{settings.effort ?? t("model.unknown")}</dd></div>
      <div><dt>{t("model.source")}</dt><dd>{sourceLabel(settings.source)}</dd></div>
    </dl>
  );
}

/**
 * The fact chip: what the session started with, `model · level`, among the
 * header's facts. It opens the facts in the path bar's popover box, as the
 * notes chip does.
 */
export function SessionModelChip({ settings }: { settings: SessionMessageSettings }) {
  const { t } = useTranslation("session");
  const label = sessionModelLabel(settings);
  return (
    <PathBarPopover
      id="session-model"
      label={t("model.chip")}
      className="session-detail-chip session-detail-model-chip"
      ariaLabel={t("model.chipLabel", { label })}
      face={settings.model != null ? <><code>{settings.model}</code> · {settings.effort ?? t("model.unknown")}</> : label}
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
  const { t } = useTranslation("session");
  const label = sessionModelLabel(settings);
  return (
    <div className="session-detail-model-note">
      <PathBarPopover
        id={`session-model:${eventId}`}
        label={t("model.note")}
        className="session-detail-model-note-chip"
        ariaLabel={t("model.noteLabel", { label })}
        face={label}
      >
        {() => <SessionModelFacts settings={settings} />}
      </PathBarPopover>
    </div>
  );
}
