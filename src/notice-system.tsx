import { StrictMode } from "react";
import { createRoot } from "react-dom/client";
import Notice, { NoticeZone, type NoticeItem } from "./components/notice/Notice";
import "./App.css";
import "./RetroTheme.css";

/**
 * The shipped notice recipe, every rung and every count, against the real
 * stylesheets. Two things are checked by looking rather than by reading source:
 * that a box FILLS its column instead of shrink-centring inside it (the defect
 * the old `.sync-callout { margin: 0 auto }` caused in every grid), and that
 * the four rungs stay legible in both palettes.
 *
 * Not part of the app bundle: the build has one entry, and this file is
 * reachable only from the dev server.
 */

const NOTES: NoticeItem[] = [
  {
    key: "cache",
    severity: "info",
    message: "Mallard rebuilt its local timeline cache because the saved cache was outdated or unreadable.",
  },
  {
    key: "preview",
    severity: "warning",
    message: "At least one Claude message exceeds the 16-KiB timeline display limit and is shown as a truncated preview.",
  },
  {
    key: "phase",
    severity: "info",
    message: "At least one Claude assistant message lacked a terminal stop reason, so its final-response placement was inferred.",
  },
];

function Frame({ label, hint, children }: { label: string; hint?: string; children: React.ReactNode }) {
  return (
    <div>
      <p className="specimen-row-label"><b>{label}</b>{hint ? <span>{hint}</span> : null}</p>
      <div className="specimen-frame">{children}</div>
    </div>
  );
}

function Specimen() {
  return (
    <div className="specimen-page">
      <header className="specimen-head">
        <div>
          <h1>Notice</h1>
          <p>
            One box per zone at the loudest rung present, the type named in the text, and the ARIA
            role derived from whether the content is standing or an event.
          </p>
        </div>
      </header>

      <section>
        <h2>The four rungs</h2>
        <p className="specimen-lead">
          Strip the tint and the glyph and the sentence still says what it is — the word before the
          middle dot is printed by the component, not written into the message.
        </p>
        <div className="specimen-rows">
          <Frame label="danger" hint="the thing failed">
            <Notice severity="danger" timing="blocking">Couldn’t open the session in Codex.</Notice>
          </Frame>
          <Frame label="warning" hint="you are seeing less than the whole truth">
            <Notice severity="warning" timing="standing" label="Notes about this session">
              At least one message is shown as a truncated preview.
            </Notice>
          </Frame>
          <Frame label="info" hint="a fact about how the data was produced">
            <Notice severity="info" timing="standing" label="Notes about this session">
              Mallard rebuilt its local timeline cache.
            </Notice>
          </Frame>
          <Frame label="success" hint="a receipt">
            <Notice severity="success" timing="progress">Pushed 12 sessions to R2 storage 1.</Notice>
          </Frame>
        </div>
      </section>

      <section>
        <h2>One zone, N messages</h2>
        <p className="specimen-lead">
          The session-detail page used to draw one amber box per warning, stacked in a grid. Because
          the box carried a page banner’s <code>margin: 0 auto</code>, the grid could not stretch
          them and each row shrank to its own sentence at a different width from the one above.
        </p>
        <div className="specimen-rows">
          <Frame label="one message" hint="no count line it does not need">
            <NoticeZone items={NOTES.slice(0, 1)} subject="notes about this session" timing="standing" label="Notes about this session" />
          </Frame>
          <Frame label="three messages" hint="one box, warning wins the tone">
            <NoticeZone items={NOTES} subject="notes about this session" timing="standing" label="Notes about this session" />
          </Frame>
          <Frame label="with an action and a failure" hint="danger wins; each message keeps its own control">
            <NoticeZone
              items={[
                { key: "failed", severity: "danger", message: "Couldn’t open the session in Codex." },
                {
                  key: "stale",
                  severity: "warning",
                  title: "This session changed.",
                  message: "Refresh before continuing so pages are not mixed across snapshots.",
                  action: <button type="button" className="btn btn-ghost">Refresh</button>,
                },
                ...NOTES,
              ]}
              subject="notes about this session"
              timing="blocking"
            />
          </Frame>
        </div>
      </section>

      <section>
        <h2>Narrow column</h2>
        <p className="specimen-lead">
          The old recipe was written as a page banner and capped itself at 1180px. This one fills
          whatever it is put in, so the same component works in the shell and in a 440px panel.
        </p>
        <div className="specimen-narrow">
          <Frame label="440px">
            <NoticeZone items={NOTES} subject="notes about this session" timing="standing" label="Notes about this session" />
          </Frame>
          <Frame label="440px, single">
            <Notice severity="danger" timing="blocking" title="Push failed.">
              The storage rejected the manifest because its revision moved.
            </Notice>
          </Frame>
        </div>
      </section>
    </div>
  );
}

createRoot(document.getElementById("root")!).render(<StrictMode><Specimen /></StrictMode>);
