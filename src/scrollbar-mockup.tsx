import { StrictMode, useEffect, useState } from "react";
import { createRoot } from "react-dom/client";
import Icon from "./components/Icons";
import "./App.css";
import "./RetroTheme.css";

/**
 * Mockup: scrollbars cutting rounded tile corners.
 *
 * Both columns render the real class names, so the right-hand column is the
 * shipped rule under test rather than a stand-in. The left column re-opens the
 * defect by resetting the track margin, which is exactly what RetroTheme used
 * to leave in place.
 *
 * Not part of the app bundle: the build has one entry, and this file is
 * reachable only from the dev server.
 */

const SHELL = `
  /* App.css locks the desktop shell with height:100%;overflow:hidden on
     html/body/#root. A mockup is a document, so it has to take that back. */
  html, body, #root { height: auto; min-height: 100%; overflow: visible; }
  body { background: var(--retro-frame-gutter); }
  .mk { max-width: 1120px; margin: 0 auto; padding: 24px 22px 70px; font-family: var(--font-sans); }
  .mk-title { margin: 0 0 4px; color: var(--text-0); font-size: var(--type-xl); font-weight: var(--weight-bold); }
  .mk-lede { margin: 0 0 16px; max-width: 84ch; color: var(--text-2); font-size: var(--type-sm); line-height: var(--leading-prose); }
  .mk-bar { display: flex; gap: 8px; align-items: center; margin: 0 0 20px; flex-wrap: wrap; }
  .mk-grid { display: grid; grid-template-columns: repeat(auto-fit, minmax(330px, 1fr)); gap: 20px; align-items: start; }
  .mk-col > h2 { margin: 0 0 3px; color: var(--text-0); font-size: var(--type-md); font-weight: var(--weight-bold); }
  .mk-col > p { margin: 0 0 12px; min-height: 40px; color: var(--text-2); font-size: var(--type-xs); line-height: var(--leading-prose); }
  .mk-stack { display: flex; flex-direction: column; gap: var(--retro-gutter); }
  /* Real .sync-workspace > .sync-main, just given a height and a lighter pad. */
  .mk-stack .sync-workspace { flex: 0 0 190px; height: 190px; }
  .mk-stack .sync-main { min-height: 0; padding: 12px 14px; }
  .mk-stack .sync-main p { margin: 0 0 12px; color: var(--text-1); font-size: var(--type-sm); line-height: var(--leading-prose); }
  .mk-stack .sidebar-storage-pane { height: 150px; }
  /* Corner detail: the tile is painted at 3x and cropped to its top-right
     corner. transform scales the painted scrollbar layer too, so this is the
     real chrome magnified, not a redrawing of it. */
  .mk-zoom { width: 210px; height: 132px; overflow: hidden; border: 1px dashed var(--border-mid);
    border-radius: 4px; background: var(--retro-frame-gutter); }
  .mk-zoom > * { width: 210px; transform: scale(3); transform-origin: 100% 0; }
  .mk-zoom .sync-workspace { flex: 0 0 110px; height: 110px; }
  .mk-cap { margin: 6px 0 0; color: var(--text-2); font-family: var(--font-mono); font-size: var(--type-2xs); }
`;

const CASES = `
  .reveal .mk-col ::-webkit-scrollbar-track { background: #b04040; }
  /* Re-opens the defect: the track runs the whole length again. */
  .as-before ::-webkit-scrollbar-track { margin: 0 !important; }
`;

const COPY = "The scrollbar is painted as a rectangle in the padding box, so its square end lands inside the corner curve.";

function Main() {
  return (
    <div className="sync-workspace">
      <main className="sync-main">
        {Array.from({ length: 14 }, (_, index) => <p key={index}>{COPY}</p>)}
      </main>
    </div>
  );
}

function Surfaces() {
  return (
    <div className="mk-stack">
      <Main />
      <div className="mk-zoom"><Main /></div>
      <p className="mk-cap">top-right corner at 3x</p>
      <section className="sidebar-split-pane sidebar-storage-pane">
        <div className="sidebar-section-heading">
          <div className="sidebar-section-label"><span className="sidebar-section-title">Storages</span></div>
        </div>
        <div className="sidebar-links-list">
          {["Local storage 1", "R2 storage 2", "R2 storage 3", "R2 archive", "R2 cold"].map((name) => (
            <div key={name} className="sidebar-profile-item">
              <button type="button" className="sidebar-profile-main">
                <Icon name="drive" size={16} /><span>{name}</span>
              </button>
            </div>
          ))}
        </div>
      </section>
    </div>
  );
}

function Mockup() {
  const [theme, setTheme] = useState<"light" | "dark">("light");
  const [reveal, setReveal] = useState(true);
  useEffect(() => { document.documentElement.dataset.theme = theme; }, [theme]);
  return (
    <div className={`mk${reveal ? " reveal" : ""}`}>
      <style>{SHELL}{CASES}</style>
      <h1 className="mk-title">Scrollbars cut the tile corners</h1>
      <p className="mk-lede">
        Both columns use the real class names. The right column is the shipped rule; the left resets the
        track margin to re-open the defect. Watch the top-right corner in the 3&times; detail, and the
        bottom-right corner of the Storages pane.
      </p>
      <div className="mk-bar">
        <button type="button" className="btn btn" onClick={() => setTheme(theme === "light" ? "dark" : "light")}>
          {theme === "light" ? "Switch to dark" : "Switch to light"}
        </button>
        <button type="button" className="btn btn" onClick={() => setReveal(!reveal)}>
          {reveal ? "Hide track tint" : "Reveal track tint"}
        </button>
      </div>
      <div className="mk-grid">
        <div className="mk-col as-before">
          <h2>Before</h2>
          <p>Track runs the full length; its square end squares off the curve it lands in.</p>
          <Surfaces />
        </div>
        <div className="mk-col">
          <h2>After — shipped rule</h2>
          <p>Track stops one inner radius short of each curve it meets. The pane list is inset at the bottom only.</p>
          <Surfaces />
        </div>
      </div>
    </div>
  );
}

createRoot(document.getElementById("root")!).render(<StrictMode><Mockup /></StrictMode>);
