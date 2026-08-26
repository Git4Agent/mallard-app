import { StrictMode, useEffect, useState } from "react";
import { createRoot } from "react-dom/client";
import Icon from "./components/Icons";
import "./App.css";
import "./RetroTheme.css";

/**
 * The proposed loading recipe, every tier, against the real stylesheets —
 * plan-stage specimen, nothing here ships yet. Two things are checked by
 * looking rather than by reading source: that a T1 scaffold has the same
 * geometry as the content that replaces it (the replay button proves the
 * no-jump claim), and that bones stay legible in both palettes and with
 * motion off.
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

function Bone({ w, h = 12, className = "" }: { w: number | string; h?: number; className?: string }) {
  return <i className={`ske ${className}`} aria-hidden="true" style={{ display: "block", width: w, height: h }} />;
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

/* ── T1 — scaffolds: the page's own bones ────────────────────────────── */

function ActivityScaffold() {
  return (
    <div className="loading-scaffold" role="region" aria-label="Loading project activity">
      <div className="loading-scaffold-caption">{RING} Loading project activity…</div>
      <Bone w={92} h={14} className="pill" />
      {[0.62, 0.44, 0.55].map((width, index) => (
        <div key={index} className="ske-row" aria-hidden="true">
          <Bone w={18} h={18} className="round quiet" />
          <div className="ske-row-lines">
            <Bone w={`${width * 100}%`} h={12} />
            <div className="ske-row-chips">
              <Bone w={54} h={9} className="pill quiet" />
              <Bone w={38} h={9} className="pill quiet" />
              <Bone w={46} h={9} className="pill quiet" />
            </div>
          </div>
          <Bone w={10} h={10} className="round quiet" />
        </div>
      ))}
    </div>
  );
}

/* The landed twin of the scaffold above, same grid and paddings, so replay
   shows content arriving without a single row shifting. */
function ActivityLanded() {
  const rows = [
    { title: "Wire the settings reset relaunch", chips: ["codex", "31 messages", "2 tools"] },
    { title: "Repair conversation paths", chips: ["claude", "12 messages", "5 tools"] },
    { title: "Catalog scope-link redesign", chips: ["codex", "48 messages", "9 tools"] },
  ];
  return (
    <div className="loading-scaffold">
      <div className="loading-scaffold-caption" style={{ color: "var(--text-3)" }}>Yesterday</div>
      <span style={{ width: 92, height: 14 }} />
      {rows.map((row) => (
        <div key={row.title} className="ske-row" style={{ background: "var(--surface-row)" }}>
          <span style={{ display: "flex", alignItems: "center", justifyContent: "center", width: 18, height: 18, color: "var(--text-2)" }}>
            <Icon name="message" size={14} />
          </span>
          <div className="ske-row-lines">
            <strong style={{ fontSize: "var(--type-md)", color: "var(--text-0)", lineHeight: "12px" }}>{row.title}</strong>
            <span style={{ display: "flex", gap: 6 }}>
              {row.chips.map((chip) => (
                <span key={chip} style={{ fontSize: "var(--type-2xs)", lineHeight: "9px", color: "var(--text-2)" }}>{chip}</span>
              ))}
            </span>
          </div>
          <span style={{ width: 10, height: 10, borderRadius: "50%", background: "var(--thread-status-synced)" }} />
        </div>
      ))}
    </div>
  );
}

function ReplayableActivity() {
  const [landed, setLanded] = useState(false);
  const [tick, setTick] = useState(0);
  useEffect(() => {
    setLanded(false);
    const timer = window.setTimeout(() => setLanded(true), 1400);
    return () => window.clearTimeout(timer);
  }, [tick]);
  return (
    <div>
      <p className="specimen-row-label">
        <b>rows — Activity first load</b>
        <span>replaces the bare “Loading project activity…” line; geometry survives the swap</span>
        <button type="button" className="btn btn-ghost" style={{ marginLeft: "auto" }} onClick={() => setTick((t) => t + 1)}>
          Replay first paint
        </button>
      </p>
      <div className="specimen-frame">{landed ? <ActivityLanded /> : <ActivityScaffold />}</div>
    </div>
  );
}

function CatalogScaffold() {
  return (
    <div className="loading-scaffold" role="region" aria-label="Loading the Library">
      <div className="loading-scaffold-caption">{RING} Loading the Library…</div>
      <div className="ske-card-grid" aria-hidden="true">
        {[0.7, 0.5, 0.8, 0.6].map((width, index) => (
          <div key={index} className="ske-card">
            <Bone w={56} h={56} />
            <Bone w={`${width * 100}%`} h={12} />
            <div className="ske-row-chips">
              <Bone w={44} h={9} className="pill quiet" />
              <Bone w={32} h={9} className="pill quiet" />
            </div>
            <Bone w="55%" h={9} className="quiet" />
          </div>
        ))}
      </div>
    </div>
  );
}

function DetailScaffold() {
  // The punchcard band: fixed tile lattice, no shimmer per tile — the band
  // shimmers as one surface so it reads as a region, not as sixty spinners.
  const tiles = Array.from({ length: 54 }, (_, index) => (index * 7) % 10);
  return (
    <div className="loading-scaffold" role="region" aria-label="Loading session detail">
      <div className="loading-scaffold-caption">{RING} Loading session detail…</div>
      <div className="ske-row-chips" aria-hidden="true">
        <Bone w={74} h={16} className="pill" />
        <Bone w={58} h={16} className="pill" />
        <Bone w={90} h={16} className="pill" />
        <Bone w={48} h={16} className="pill" />
      </div>
      <div className="ske-band ske" aria-hidden="true" style={{ background: "var(--surface-card)" }}>
        {tiles.map((step, index) => (
          <i key={index} style={{ opacity: 0.25 + step * 0.06 }} />
        ))}
      </div>
      <div className="specimen-pair" aria-hidden="true" style={{ gap: 10 }}>
        {[0, 1].map((pane) => (
          <div key={pane} className="ske-pane">
            <Bone w={64} h={9} className="pill quiet" />
            <Bone w="92%" h={11} />
            <Bone w="78%" h={11} />
            <Bone w="84%" h={11} />
          </div>
        ))}
      </div>
    </div>
  );
}

function SettingsScaffold() {
  return (
    <div className="loading-scaffold" role="region" aria-label="Loading settings">
      <div className="loading-scaffold-caption">{RING} Loading settings…</div>
      {[0, 1].map((group) => (
        <div key={group} className="ske-pane" aria-hidden="true" style={{ gap: 0 }}>
          <Bone w={110} h={11} className="pill quiet" />
          {[0.34, 0.46].map((width, index) => (
            <div key={index} className="ske-field">
              <div className="ske-row-lines">
                <Bone w={`${width * 100}%`} h={11} />
                <Bone w="62%" h={9} className="quiet" />
              </div>
              <Bone w={190} h={24} />
            </div>
          ))}
        </div>
      ))}
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
            Plan-stage specimen. Four tiers, one clock, and the page's own bones instead of a bare
            line: every read names what it is loading, every write keeps speaking through the
            NoticeZone, and nothing below ships yet.
          </p>
        </div>
        <div className="specimen-controls">
          <label><input type="checkbox" checked={dark} onChange={(event) => setDark(event.target.checked)} /> dark</label>
          <label><input type="checkbox" checked={reduce} onChange={(event) => setReduce(event.target.checked)} /> reduced motion</label>
        </div>
      </header>

      <section>
        <h2>What ships today</h2>
        <p className="specimen-lead">
          Thirteen loading surfaces, three spinner motifs on three different clocks (0.8s icon,
          0.85s ring, 1.8s glint), and no skeletons anywhere: a page's first load is a single line
          on an otherwise blank panel, so content lands with a full-layout jump. The catalog's line
          has no loader at all. The table is the inventory the recipe consolidates — the same move
          the Notice recipe made on 44 callout sites.
        </p>
        <table className="specimen-table">
          <thead><tr><th>Surface</th><th>Today</th><th>Recipe tier</th></tr></thead>
          <tbody>
            <tr><td>App startup</td><td>brand mark, staged 2px track, status line (<code>StartupLoadingScreen</code>)</td><td>T0 — unchanged; lends its clock</td></tr>
            <tr><td>Activity first load</td><td>ring + “Loading project activity…” on a blank panel</td><td>T1 rows</td></tr>
            <tr><td>Library first load</td><td>“Loading ⟨scope⟩…”, no loader (<code>StorageCatalogPage</code>)</td><td>T1 cards</td></tr>
            <tr><td>Session detail entry</td><td>ring + heading + subline, centred</td><td>T1 detail</td></tr>
            <tr><td>Settings groups</td><td>three independent flags, values pop in</td><td>T1 form</td></tr>
            <tr><td>Timeline page fetch</td><td>ring + “Loading the filtered timeline…” inside the drawn detail</td><td>T2 — kept as is</td></tr>
            <tr><td>Log panel, repositories</td><td>ring + named line</td><td>T2 — kept as is</td></tr>
            <tr><td>Thread compare badges</td><td>“Comparing with ⟨storage⟩…” per row</td><td>T2 — kept as is</td></tr>
            <tr><td>Buttons and icon actions</td><td>spinning refresh + progressive verb; <code>.loading:disabled</code></td><td>T3 — kept, retimed</td></tr>
            <tr><td>Push / Pull in flight</td><td>NoticeZone <code>timing: progress</code></td><td>out of scope — writes advise, they do not “load”</td></tr>
          </tbody>
        </table>
      </section>

      <section>
        <h2>The rules</h2>
        <p className="specimen-rule"><b>R1 — one clock.</b> The startup glint's 1.8s ease-in-out is the shimmer everywhere; rings and spinning icons run its half-period, 0.9s. Splash-to-page then reads as one continuous instrument, not three gadgets.</p>
        <p className="specimen-rule"><b>R2 — name the thing.</b> Every line is “Loading ⟨noun⟩…” with the concrete noun; a busy control wears the progressive verb of its own label (“Scanning…”, “Preparing…”). Bare “Loading…” survives only where the resting label already named the noun (load-more).</p>
        <p className="specimen-rule"><b>R3 — draw the destination.</b> A page's first read renders its own layout in bones — same containers, same paddings — so arrival changes pixels, never geometry. Four shapes cover every page: rows, cards, detail, form.</p>
        <p className="specimen-rule"><b>R4 — the ARIA stays as designed.</b> First paint is quiet (<code>role="region"</code>, bones aria-hidden); user-triggered refreshes keep their polite announcements; a failure replaces the scaffold with the alert or blocking Notice it already has. No live regions are added.</p>
        <p className="specimen-rule"><b>R5 — reads load, writes advise.</b> Push, Pull, and repair keep speaking through the NoticeZone's progress timing. The loading recipe never narrates a mutation.</p>
        <p className="specimen-rule"><b>R6 — motion is optional.</b> Reduced motion stops every clock: bones hold their two-tone fill, the ring keeps its ¾-arc shape, and “busy” still reads by form alone. Toggle it above.</p>
        <p className="specimen-rule"><b>R7 — the 150ms gate.</b> No T1/T2 state appears before 150ms. The observation disk cache made warm loads near-instant; the gate keeps them flash-free while cold loads still get bones immediately enough.</p>
      </section>

      <section>
        <h2>T0 — splash</h2>
        <p className="specimen-lead">Shipped and unchanged: the only place the duck loads. Reproduced here so its clock and track can be compared against the tiers below.</p>
        <Frame label="startup" hint="stage 4 of 6, glint riding the filled track" tight>
          <Splash />
        </Frame>
      </section>

      <section>
        <h2>T1 — scaffolds</h2>
        <p className="specimen-lead">
          The enhancement. One caption line (ring + named noun) above the page's own bones; the
          bones shimmer on the shared clock. Each scaffold is the geometry of its landed page.
        </p>
        <div className="specimen-rows">
          <ReplayableActivity />
          <Frame label="cards — the Library" hint="pet tile square, name, kind chips; replaces the loader-less line">
            <CatalogScaffold />
          </Frame>
          <Frame label="detail — session detail" hint="fact-chip strip, punchcard band shimmering as one surface, stacked panes">
            <DetailScaffold />
          </Frame>
          <Frame label="form — settings" hint="group panes with label/control pairs; three flags become one scaffold">
            <SettingsScaffold />
          </Frame>
        </div>
      </section>

      <section>
        <h2>T2 — lines · T3 — controls</h2>
        <p className="specimen-lead">
          Kept, codified, retimed. A line is for panes too small or shapeless for bones; a busy
          control spins its own icon and conjugates its own label.
        </p>
        <div className="specimen-pair">
          <Frame label="T2 line" hint="shipped .sync-history-state">
            <div className="sync-history-state">{RING} Loading retained logs…</div>
          </Frame>
          <Frame label="T3 controls" hint="shipped .btn + .icon-spin">
            <div style={{ display: "flex", gap: 10, flexWrap: "wrap", alignItems: "center" }}>
              <button type="button" className="btn" disabled>
                <Icon name="refresh" size={14} className="icon-spin" /> Scanning…
              </button>
              <button type="button" className="btn btn-primary" disabled>
                <Icon name="refresh" size={14} className="icon-spin" /> Preparing…
              </button>
              <button type="button" className="btn btn-ghost" disabled>Loading…</button>
              <span style={{ display: "inline-flex", alignItems: "center", gap: 6, fontSize: "var(--type-xs)", color: "var(--text-2)" }}>
                {RING} Comparing with Shared R2…
              </span>
            </div>
          </Frame>
        </div>
      </section>

      <section>
        <h2>Wording</h2>
        <table className="specimen-table">
          <thead><tr><th>Site</th><th>Today</th><th>Recipe</th></tr></thead>
          <tbody>
            <tr><td>Activity</td><td>“Loading project activity…”</td><td>unchanged — already names the noun</td></tr>
            <tr><td>Library</td><td>“Loading ⟨scope label⟩…”</td><td>unchanged wording, gains the ring and bones</td></tr>
            <tr><td>Session detail entry</td><td>“Loading session detail…” + “Preparing the local, bounded timeline.”</td><td>caption keeps the first line; the subline retires — bones carry the promise</td></tr>
            <tr><td>Timeline page</td><td>“Loading the filtered timeline…”</td><td>unchanged (T2 inside a drawn page)</td></tr>
            <tr><td>Settings</td><td>values pop in silently</td><td>“Loading settings…” caption over the form scaffold</td></tr>
            <tr><td>Buttons</td><td>“Scanning…” / “Retrying…” / “Preparing…” / “Loading…”</td><td>unchanged — already the rule</td></tr>
          </tbody>
        </table>
      </section>
    </div>
  );
}

createRoot(document.getElementById("root")!).render(
  <StrictMode>
    <Specimen />
  </StrictMode>,
);
