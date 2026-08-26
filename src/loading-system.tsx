import { StrictMode, useEffect, useState } from "react";
import { createRoot } from "react-dom/client";
import Icon from "./components/Icons";
import LoadingScaffold, { LoadingBone } from "./components/project-sync/LoadingScaffold";
import "./App.css";
import "./RetroTheme.css";

/**
 * The shipped loading recipe, every tier, against the real stylesheets and
 * the real LoadingScaffold component. Two things are checked by looking
 * rather than by reading source: that a T1 scaffold has the same geometry
 * as the content that replaces it (the replay button proves the no-jump
 * claim), and that bones stay legible in both palettes and with motion
 * off. Scaffolds here appear ~150ms after mount — that is the shipped R7
 * gate doing its job, not a defect.
 *
 * Not part of the app bundle: the build has one entry, and this file is
 * reachable only from the dev server.
 */

const RING = <span className="status-loader" />;

function Frame({ label, hint, children, tight }: {
  label: string; hint?: string; children: React.ReactNode; tight?: boolean;
}) {
  return (
    <div>
      <p className="specimen-row-label"><b>{label}</b>{hint ? <span>{hint}</span> : null}</p>
      <div className={`specimen-frame${tight ? " tight" : ""}`}>{children}</div>
    </div>
  );
}

/* ── T0 — the splash, as shipped ─────────────────────────────────────── */

function Splash() {
  return (
    <main className="sync-startup-screen">
      <section className="sync-startup-content">
        <div className="sync-startup-brand">
          <div className="sync-startup-mark" aria-hidden="true">
            <img src="/mallard-logo-duck.svg" alt="" />
          </div>
          <h1>Mallard</h1>
        </div>
        <div className="sync-startup-progress">
          <div
            className="sync-startup-progress-track"
            role="progressbar"
            aria-label="Mallard startup progress"
            style={{ "--sync-startup-progress": "64%" } as React.CSSProperties}
          >
            <span />
          </div>
          <p role="status">Finding projects…</p>
        </div>
      </section>
    </main>
  );
}

/* The landed twin of the rows scaffold — the thread-card anatomy with real
   ink, same paddings and gaps, so replay shows content arriving without a
   single row shifting. */
function ActivityLanded() {
  const rows = [
    { title: "Wire the settings reset relaunch", chips: ["codex", "31 messages", "2 tools"] },
    { title: "Repair conversation paths", chips: ["claude", "12 messages", "5 tools"] },
    { title: "Catalog scope-link redesign", chips: ["codex", "48 messages", "9 tools"] },
  ];
  return (
    <div className="loading-scaffold">
      <div className="loading-scaffold-caption" style={{ color: "var(--text-3)" }}>Yesterday</div>
      <div className="loading-scaffold-list">
        {rows.map((row) => (
          <div key={row.title} className="loading-scaffold-row">
            <div className="loading-scaffold-topline">
              <div className="loading-scaffold-title">
                <span style={{ width: 12, height: 12, borderRadius: "50%", background: "var(--thread-status-synced)", flex: "0 0 auto" }} />
                <strong style={{ fontSize: "var(--type-md)", color: "var(--text-0)", lineHeight: "13px" }}>{row.title}</strong>
              </div>
              <div className="loading-scaffold-actions" style={{ color: "var(--text-2)" }}>
                <Icon name="message" size={14} />
                <Icon name="git-fork" size={14} />
                <Icon name="folder" size={14} />
              </div>
            </div>
            <div className="loading-scaffold-chips" style={{ gap: 14 }}>
              {row.chips.map((chip) => (
                <span key={chip} style={{ fontSize: "var(--type-2xs)", lineHeight: "9px", color: "var(--text-2)" }}>{chip}</span>
              ))}
            </div>
          </div>
        ))}
      </div>
    </div>
  );
}

function ReplayableActivity() {
  const [landed, setLanded] = useState(false);
  const [tick, setTick] = useState(0);
  useEffect(() => {
    setLanded(false);
    const timer = window.setTimeout(() => setLanded(true), 1600);
    return () => window.clearTimeout(timer);
  }, [tick]);
  return (
    <div>
      <p className="specimen-row-label">
        <b>rows — Activity first load</b>
        <span>the shipped scaffold; geometry survives the swap</span>
        <button type="button" className="btn btn-ghost" style={{ marginLeft: "auto" }} onClick={() => setTick((t) => t + 1)}>
          Replay first paint
        </button>
      </p>
      <div className="specimen-frame">
        {landed
          ? <ActivityLanded />
          : <LoadingScaffold key={tick} shape="rows" label="Loading project activity" />}
      </div>
    </div>
  );
}

/* ── The page ────────────────────────────────────────────────────────── */

function Specimen() {
  const [dark, setDark] = useState(false);
  const [reduce, setReduce] = useState(false);
  useEffect(() => {
    document.documentElement.setAttribute("data-theme", dark ? "dark" : "light");
  }, [dark]);

  return (
    <div className={`specimen-page${reduce ? " specimen-reduce" : ""}`}>
      <header className="specimen-head">
        <div>
          <h1>Loading — one recipe</h1>
          <p>
            SHIPPED. Four tiers, one clock, and the page's own bones instead of a bare line: every
            read names what it is loading, and every write keeps speaking through the NoticeZone.
            The scaffolds below are the real <code>LoadingScaffold</code> component.
          </p>
        </div>
        <div className="specimen-controls">
          <label><input type="checkbox" checked={dark} onChange={(event) => setDark(event.target.checked)} /> dark</label>
          <label><input type="checkbox" checked={reduce} onChange={(event) => setReduce(event.target.checked)} /> reduced motion</label>
        </div>
      </header>

      <section>
        <h2>Where each tier landed</h2>
        <table className="specimen-table">
          <thead><tr><th>Surface</th><th>Was</th><th>Now</th></tr></thead>
          <tbody>
            <tr><td>App startup</td><td>brand mark, staged 2px track, status line</td><td>T0 — unchanged; lends its 1.8s clock</td></tr>
            <tr><td>Activity first load</td><td>ring + line on a blank panel</td><td>T1 rows scaffold (<code>ProjectActivityPage</code>)</td></tr>
            <tr><td>Library first load</td><td>loader-less “Loading ⟨scope⟩…” line</td><td>T1 rows scaffold, same wording as caption — its landed list is the same thread-card family as Activity's, so a card grid would have advertised the wrong destination (<code>StorageCatalogPage</code>)</td></tr>
            <tr><td>Session detail entry</td><td>centred heading + “Preparing the local, bounded timeline.”</td><td>T1 detail scaffold; the subline retired — geometry carries the promise</td></tr>
            <tr><td>Settings</td><td>plausible defaults while values load; readout showed “…”</td><td>value-level <code>LoadingBone</code> in the Local data readout only — the groups render settled structure with disabled controls from first paint, so a page scaffold would have hidden ready content (the plan's form shape was cut as unused)</td></tr>
            <tr><td>Timeline page fetch, log panel, repositories, compare badges</td><td>ring + named line</td><td>T2 — kept as is</td></tr>
            <tr><td>Buttons and icon actions</td><td>0.8s spin</td><td>T3 — kept, retimed to 0.9s (ring 0.85s → 0.9s too)</td></tr>
            <tr><td>Push / Pull in flight</td><td colSpan={2}>NoticeZone <code>timing: progress</code> — writes advise, they do not “load”</td></tr>
          </tbody>
        </table>
      </section>

      <section>
        <h2>The rules</h2>
        <p className="specimen-rule"><b>R1 — one clock.</b> 1.8s ease-in-out shimmer; rings and spinning icons at the 0.9s half-period.</p>
        <p className="specimen-rule"><b>R2 — name the thing.</b> “Loading ⟨noun⟩…” always; busy controls conjugate their own label.</p>
        <p className="specimen-rule"><b>R3 — draw the destination.</b> Bones mirror landed geometry; arrival changes pixels, never layout.</p>
        <p className="specimen-rule"><b>R4 — the ARIA stays as designed.</b> Quiet named region at first paint, bones aria-hidden, no new live regions; failures keep their alert/blocking Notices.</p>
        <p className="specimen-rule"><b>R5 — reads load, writes advise.</b> The NoticeZone owns in-flight mutations.</p>
        <p className="specimen-rule"><b>R6 — motion is optional.</b> The app's global reduced-motion block freezes every clock; bones hold their two-tone fill. Toggle above.</p>
        <p className="specimen-rule"><b>R7 — the 150ms gate.</b> A scaffold renders nothing for its first 150ms, so cache-warm loads never flash it.</p>
      </section>

      <section>
        <h2>T0 — splash</h2>
        <Frame label="startup" hint="stage 4 of 6, glint riding the filled track" tight>
          <Splash />
        </Frame>
      </section>

      <section>
        <h2>T1 — scaffolds</h2>
        <div className="specimen-rows">
          <ReplayableActivity />
          <Frame label="rows — the Library" hint="the same thread-card family Activity lands; badge, name, action run, meta line (StorageCatalogPage)">
            <LoadingScaffold shape="rows" label="Loading Skills · Default Codex" />
          </Frame>
          <Frame label="detail — session detail" hint="fact-chip strip, punchcard band shimmering as one surface, stacked panes">
            <LoadingScaffold shape="detail" label="Loading session detail" />
          </Frame>
        </div>
      </section>

      <section>
        <h2>T2 — lines · T3 — controls · value bones</h2>
        <div className="specimen-pair">
          <Frame label="T2 line" hint="shipped .sync-history-state">
            <div className="sync-history-state">{RING} Loading retained logs…</div>
          </Frame>
          <Frame label="T3 controls + a value bone" hint="shipped .btn + .icon-spin; Settings' Local data readout">
            <div style={{ display: "flex", gap: 10, flexWrap: "wrap", alignItems: "center" }}>
              <button type="button" className="btn" disabled>
                <Icon name="refresh" size={14} className="icon-spin" /> Scanning…
              </button>
              <button type="button" className="btn btn-primary" disabled>
                <Icon name="refresh" size={14} className="icon-spin" /> Preparing…
              </button>
              <span style={{ display: "inline-flex", alignItems: "center", gap: 6, fontSize: "var(--type-xs)", color: "var(--text-2)" }}>
                {RING} Comparing with Shared R2…
              </span>
              <span className="sync-settings-readout"><LoadingBone width={110} height={11} /></span>
            </div>
          </Frame>
        </div>
      </section>
    </div>
  );
}

createRoot(document.getElementById("root")!).render(
  <StrictMode>
    <Specimen />
  </StrictMode>,
);
