import { StrictMode, useState } from "react";
import { createRoot, type Root } from "react-dom/client";
import SyncOutcomeNotice, { type SyncOutcomeRowView, type SyncOutcomeView } from "./components/sync-review/SyncOutcomeNotice";
import "./App.css";
import "./RetroTheme.css";

/**
 * Specimen for the Pull-complete banner's row titles, as shipped.
 *
 * The row is named by the stored thread's own title, carried onto the plan by
 * ConversationReviewRow.display_title. When the stored thread has no title the
 * row reads "Untitled session" and the whole identifier rides line 2 -- it is
 * never named after its id, because a CloudThreadId's first characters are the
 * mandatory "cloudthread-" namespace and every untitled cloud row would
 * therefore carry the identical name.
 *
 * The logic is covered by tests (tests/frontend/pull-review.integration.test.tsx
 * "a restored row is named by its stored title, or reads untitled"); this file
 * exists to show the result under the shipped stylesheets.
 *
 * Not part of the app bundle: the build has one entry, and this file is
 * reachable only from the dev server.
 */

const CLOUD_A = "cloudthread-1d9b172be32c17009654d805";
const CLOUD_B = "cloudthread-787c5e05601fee3e016f3514";
const LOCAL = "019f7798-5437-7632-9dbc-5b589cf68bf0";

type OptionId = "titled" | "untitled";

const OPTIONS: { id: OptionId; label: string; note: string }[] = [
  {
    id: "titled",
    label: "Stored title present",
    note: "The common case: the thread carries a display_title in the manifest, it rides the review row onto the plan, and the banner prints it \u2014 the same name the review screen above showed.",
  },
  {
    id: "untitled",
    label: "No stored title",
    note: "The thread has no title at all. The row says so plainly and line 2 carries the whole identifier. Before the fix this row read \u201cTask cloudthr\u201d \u2014 identical on every untitled cloud row, since slice(0, 8) never escapes the 12-character namespace prefix.",
  },
];

function rowsFor(option: OptionId): SyncOutcomeRowView[] {
  const synced = { icon: "check-circle" as const, className: "synced", label: "Synced" };
  return [
    {
      key: CLOUD_A,
      title: option === "titled" ? "Can you create a pelican-riding-a-bicycle svg" : "Untitled session",
      presentation: synced,
      identifier: CLOUD_A,
    },
    {
      key: CLOUD_B,
      title: option === "titled" ? "Add horror story document (Preserved 1 \u00b7 Host 6869)" : "Untitled session",
      presentation: synced,
      identifier: CLOUD_B,
    },
    {
      key: "local",
      title: "Enhance the game loop",
      presentation: synced,
      identifier: LOCAL,
    },
  ];
}

function viewFor(option: OptionId): SyncOutcomeView {
  return {
    tone: "success",
    title: "Pull complete",
    stats: [
      { icon: "download", value: "2", label: "restored" },
      { icon: "git-branch", label: "Revision", value: "2", lead: true },
    ],
    rows: rowsFor(option),
    action: null,
    defaultOpen: true,
    technical: [],
  };
}

function Harness() {
  const [dark, setDark] = useState(
    () => document.documentElement.getAttribute("data-theme") === "dark",
  );

  const toggleTheme = () => {
    const next = !dark;
    setDark(next);
    document.documentElement.setAttribute("data-theme", next ? "dark" : "light");
  };

  return (
    <div className="app mockup-root">
      <style>{SHELL_CSS}</style>
      <div className="mockup-shell">
        <div className="mockup-head">
          <div>
            <h1>Pull banner &mdash; row names</h1>
            <p>
              The real <code>SyncOutcomeNotice</code> under the shipped stylesheets, with the two
              cloud thread ids from the report. A row is named by its stored title; with no stored
              title it reads &ldquo;Untitled session&rdquo; and never invents a name from its id.
            </p>
          </div>
          <div className="mockup-controls">
            <button type="button" className="btn btn-ghost" onClick={toggleTheme}>{dark ? "Light" : "Dark"}</button>
          </div>
        </div>

        {OPTIONS.map((option) => (
          <section key={option.id} className="mockup-panel">
            <h3>{option.label}</h3>
            <p>{option.note}</p>
            <div className="mockup-stage">
              <SyncOutcomeNotice view={viewFor(option.id)} onDismiss={() => undefined} />
            </div>
          </section>
        ))}
      </div>
    </div>
  );
}

const SHELL_CSS = `
/* Same unlock as the other specimens: the app height-locks the chain and
   scrolls inner panes, a specimen is a document and scrolls itself. */
html, body { height: auto; min-height: 100%; overflow: visible; }
#root { height: auto; min-height: 100%; overflow: visible; }
.app.mockup-root { height: auto; min-height: 100vh; overflow: visible; }

.mockup-shell { width: 100%; max-width: 1000px; margin: 0 auto; padding: 28px 24px 90px; }
.mockup-head { display: flex; align-items: flex-start; justify-content: space-between; gap: 24px; margin-bottom: 8px; }
.mockup-head h1 { margin: 0 0 6px; color: var(--text-0); font-family: var(--font-pixel); font-size: var(--type-xl); font-weight: 400; }
.mockup-head p { max-width: 76ch; margin: 0; color: var(--text-2); font-size: var(--type-md); line-height: 1.55; }
.mockup-head p code, .mockup-panel > p code { font-family: var(--font-mono); font-size: 0.92em; }
.mockup-controls { display: flex; flex: 0 0 auto; gap: 8px; }
.mockup-panel { margin-top: 30px; padding-top: 18px; border-top: 1px solid var(--border); }
.mockup-panel > h3 { margin: 0 0 4px; color: var(--text-0); font-family: var(--font-pixel); font-size: var(--type-lg); font-weight: 400; }
.mockup-panel > p { max-width: 84ch; margin: 0 0 14px; color: var(--text-2); font-size: var(--type-sm); line-height: 1.55; }
/* The banner is a section of the storage card and draws no outline of its own,
   so the stage supplies the card edge it normally sits inside. */
.mockup-stage { border: 1px solid var(--border-mid); border-radius: 4px; background: var(--surface-row); overflow: hidden; }
body { background: var(--app-shell); }
`;

/* ?theme=dark so a headless capture can take the dark panel without clicking. */
const START_DARK = new URLSearchParams(location.search).get("theme") === "dark";
document.documentElement.setAttribute("data-theme", START_DARK ? "dark" : "light");
const container = document.getElementById("root")! as HTMLElement & { _root?: Root };
container._root ??= createRoot(container);
container._root.render(<StrictMode><Harness /></StrictMode>);
