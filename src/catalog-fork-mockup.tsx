import { StrictMode, useEffect, useRef, useState, type FormEvent, type ReactNode } from "react";
import { createRoot, type Root } from "react-dom/client";
import Icon from "./components/Icons";
import ProviderMark from "./components/sync-review/ProviderMark";
import ThreadStatusIcon, { type ThreadStatusIconName } from "./components/sync-review/ThreadStatusIcon";
import { PET_THUMBS } from "./catalog-pet-mockup-thumbs";
import "./App.css";
import "./RetroTheme.css";

/**
 * Specimen for the fork drawer: the Catalog card's inline Fork form redrawn as
 * a full-width drawer under the card's own lines. The shipped form has two
 * defects this page demonstrates and fixes:
 *
 *  1. Geometry — `.catalog-inline-form` carries no grid placement, so on a pet
 *     card it auto-places into the `preview` column of the card's template and
 *     inflates that `auto` column to the form's own width, shoving the title
 *     and meta to mid-card (the screenshot that started this). The drawer
 *     spans `grid-column: 1 / -1`.
 *  2. Grammar — the suggested name was a *placeholder* built from the display
 *     name ("Mallard Duck-local-copy"), which the backend rejects: fork names
 *     are component names ([A-Za-z0-9._-], alphanumeric start). The drawer
 *     prefills a valid seed from the directory name, pre-selected, checked as
 *     you type, with the rename form's own button grammar (primary Fork,
 *     ghost Cancel, inline refusal).
 *
 * Cards are hand-built DOM wearing the shipped classes over the shipped
 * stylesheets; the drawer ships in StorageCatalogPage + App.css now, and this
 * page keeps the before/after side by side. Not part of the app bundle — dev
 * server only.
 */

// ---------------------------------------------------------------------------
// The proposed drawer
// ---------------------------------------------------------------------------

/** Client-side mirror of `validate_component_name`: ASCII alphanumeric start,
 *  then letters, digits, dots, dashes, underscores; no trailing dot. The
 *  backend stays the authority — this only catches the obvious while typing. */
function nameRuleBroken(value: string): boolean {
  if (value === "") return false;
  return !/^[A-Za-z0-9][A-Za-z0-9._-]*$/.test(value) || value.endsWith(".");
}

const RULE_MESSAGE =
  "Letters and digits, plus dots, dashes and underscores after the first character — this becomes the copy’s folder name.";

function ForkDrawer({
  displayName,
  seed,
  maxLength,
  serverError,
  onClose,
}: {
  displayName: string;
  /** Seeded from the row's directory name — valid by construction — never the
   *  free-text display name. */
  seed: string;
  maxLength: number;
  /** A refusal from prepare/apply, shown where the hint line was. */
  serverError?: string;
  onClose: () => void;
}) {
  const [name, setName] = useState(seed);
  const [busy, setBusy] = useState(false);
  const inputRef = useRef<HTMLInputElement>(null);

  /* Prefilled and pre-selected: Enter forks under the suggestion, first
     keystroke replaces it wholesale. */
  useEffect(() => {
    inputRef.current?.select();
  }, []);

  const trimmed = name.trim();
  const broken = nameRuleBroken(trimmed);

  const submit = (event: FormEvent) => {
    event.preventDefault();
    if (busy || broken || trimmed === "") return;
    setBusy(true);
    window.setTimeout(() => {
      setBusy(false);
      onClose();
    }, 700);
  };

  return (
    <form className="catalog-fork-drawer" onSubmit={submit}>
      <span className="catalog-fork-lead">
        Fork <strong>“{displayName}”</strong> as
      </span>
      <input
        ref={inputRef}
        value={name}
        maxLength={maxLength}
        spellCheck={false}
        autoCapitalize="off"
        autoCorrect="off"
        aria-label={`Name for the fork of ${displayName}`}
        aria-invalid={broken || undefined}
        readOnly={busy}
        onChange={(event) => setName(event.target.value)}
      />
      <button type="submit" className="btn btn-primary" disabled={busy || broken || trimmed === ""}>
        <Icon name="git-fork" size={13} /> Fork
      </button>
      <button type="button" className="btn btn-ghost" disabled={busy} onClick={onClose}>
        Cancel
      </button>
      {broken || serverError ? (
        <span className="catalog-fork-note danger" role="alert">
          <Icon name="alert-triangle" size={13} /> {broken ? RULE_MESSAGE : serverError}
        </span>
      ) : (
        <span className="catalog-fork-note">
          An independent copy beside the original, under this name — its own row, only on this
          computer until you push it.
        </span>
      )}
    </form>
  );
}

// ---------------------------------------------------------------------------
// The card, wearing the shipped classes
// ---------------------------------------------------------------------------

function SpecimenCard({
  display,
  status,
  statusLabel,
  preview,
  petDescription,
  files,
  size,
  stamp,
  stampDetail,
  kindPhrase,
  forkOpen,
  onFork,
  children,
}: {
  display: string;
  status: ThreadStatusIconName;
  statusLabel: string;
  /** A data URI draws the pet tile; undefined draws a skill card (no column). */
  preview?: string;
  petDescription?: string;
  files: string;
  size: string;
  stamp: string;
  stampDetail: string;
  kindPhrase: string;
  forkOpen: boolean;
  onFork?: () => void;
  children?: ReactNode;
}) {
  return (
    <article className={`sync-history-thread-card catalog-card${preview !== undefined ? " catalog-card-pet" : ""}`}>
      {preview !== undefined && (
        <span className="catalog-pet-preview" data-tooltip={petDescription} tabIndex={petDescription ? 0 : undefined}>
          <img src={preview} alt={`${display} preview`} />
        </span>
      )}
      <div className="sync-history-thread-topline">
        <div className="sync-history-thread-title">
          <span className="sync-thread-sync-indicator local" data-tooltip={statusLabel} aria-label={statusLabel} tabIndex={0}>
            <ThreadStatusIcon status={status} size={18} />
          </span>
          <strong title={display}>{display}</strong>
        </div>
        <div className="sync-history-thread-actions">
          <span className="sync-history-launch-action sync-history-provider-inert" title={kindPhrase}>
            <ProviderMark provider="codex" displayName="Codex" />
          </span>
          <button
            type="button"
            className="btn btn-ghost sync-history-launch-action"
            title={`Fork ${display} — duplicate this ${kindPhrase} under a new name`}
            aria-label={`Fork ${display} — duplicate this ${kindPhrase} under a new name`}
            aria-expanded={forkOpen}
            onClick={onFork}
          >
            <Icon name="git-fork" size={14} />
          </button>
          <button
            type="button"
            className="btn btn-ghost sync-history-launch-action"
            title={`Reveal ${display} in Finder`}
            aria-label={`Reveal ${display} in Finder`}
          >
            <Icon name="folder" size={14} />
          </button>
        </div>
      </div>
      <div className="sync-history-session-details">
        <div className="sync-history-session-summary">
          <div className="sync-history-thread-meta" aria-label={`${display} details`}>
            <span className="sync-history-thread-metric" data-tooltip={`Files · ${files}`} tabIndex={0}>
              <Icon name="file" size={12} /><b>{files}</b>
            </span>
            <span className="sync-history-thread-metric" data-tooltip={`On disk · ${size}`} tabIndex={0}>
              <Icon name="drive" size={12} /><b>{size}</b>
            </span>
            <span className="sync-history-thread-metric" aria-hidden="true" />
          </div>
          <span className="sync-history-actions-divider" aria-hidden="true" />
          <div className="sync-history-row-clickables">
            <div className="sync-history-debug-menu">
              <button type="button" className="btn btn-ghost sync-history-session-detail-action" title="Paths and digests">
                <Icon name="info" size={13} />
              </button>
            </div>
          </div>
          <span
            className="sync-history-thread-metric sync-history-thread-time sync-history-thread-time-end"
            data-tooltip={stampDetail}
            aria-label={stampDetail}
            tabIndex={0}
          >
            <time>{stamp}</time>
          </span>
        </div>
      </div>
      {children}
    </article>
  );
}

// ---------------------------------------------------------------------------
// The harness
// ---------------------------------------------------------------------------

function Panel({ title, note, children }: { title: string; note: ReactNode; children: ReactNode }) {
  return (
    <section className="mockup-panel">
      <h3>{title}</h3>
      <p>{note}</p>
      <div className="mockup-stage">{children}</div>
    </section>
  );
}

function SectionList({ heading, icon, children }: { heading: string; icon: "openai" | "file"; children: ReactNode }) {
  return (
    <section className="profile-links-section" aria-label={heading}>
      <div className="sync-history-section-heading">
        <h2><Icon name={icon} size={14} />{heading}</h2>
      </div>
      <div className="sync-history-thread-list">{children}</div>
    </section>
  );
}

function Harness() {
  const [dark, setDark] = useState(
    () => document.documentElement.getAttribute("data-theme") === "dark",
  );
  /* One drawer at a time is the page's rule; each panel here keeps its own so
     both open specimens are visible without clicking. */
  const [openId, setOpenId] = useState<string | null>("maple");
  const toggle = (id: string) => setOpenId((current) => (current === id ? null : id));
  const [openSkillId, setOpenSkillId] = useState<string | null>("api-designer");
  const toggleSkill = (id: string) => setOpenSkillId((current) => (current === id ? null : id));

  return (
    <div className="app mockup-root">
      <div className="mockup-shell">
        <header className="mockup-head">
          <div>
            <h1>Fork on the card — the drawer</h1>
            <p>
              The inline Fork form, redrawn. Today it is a bordered box that auto-places into the
              pet card’s preview column and inflates it, its label says “Fork local resource”, and
              the suggested name is a placeholder built from the display name — which the backend
              refuses whenever that name isn’t a valid folder name. The drawer below spans the
              card, reads as one sentence, prefills a valid name from the directory, and refuses
              bad names while you type.
            </p>
          </div>
          <button
            type="button"
            className="btn btn-ghost"
            onClick={() => {
              const next = !dark;
              setDark(next);
              document.documentElement.setAttribute("data-theme", next ? "dark" : "light");
            }}
          >
            <Icon name={dark ? "sun" : "moon"} size={13} /> {dark ? "Light" : "Dark"}
          </button>
        </header>

        <Panel
          title="The drawer, where the box was"
          note={<>
            Full width under the card’s own lines, parted by the card’s hairline — no nested box.
            The name is prefilled from the folder name and pre-selected, so Enter forks the
            suggestion and the first keystroke replaces it. Fork is the primary button, the way
            Save is on the rename row; the fork icon on the title line shows pressed while its
            drawer is open. Click it to close and reopen.
          </>}
        >
          <SectionList heading="Pets v2" icon="openai">
            <SpecimenCard
              display="Maple"
              status="local-outstanding"
              statusLabel="Only on this computer. Push to save it in Vault."
              preview={PET_THUMBS.maple}
              petDescription="A border collie with a taste for order."
              files="1"
              size="2.5 MB"
              stamp="Jul 11, 2026 at 00:55"
              stampDetail="Last modified on this Mac · Jul 11, 2026 at 00:55"
              kindPhrase="Codex pet v2"
              forkOpen={openId === "maple"}
              onFork={() => toggle("maple")}
            >
              {openId === "maple" && (
                <ForkDrawer displayName="Maple" seed="maple-copy" maxLength={64} onClose={() => setOpenId(null)} />
              )}
            </SpecimenCard>
            <SpecimenCard
              display="Dahuang"
              status="synced"
              statusLabel="Up to date here and in Vault."
              preview={PET_THUMBS.dahuang}
              petDescription="A big yellow dog who has seen things."
              files="1"
              size="1.8 MB"
              stamp="Jun 2, 2026 at 18:20"
              stampDetail="Last modified on this Mac · Jun 2, 2026 at 18:20"
              kindPhrase="Codex pet v2"
              forkOpen={openId === "dahuang"}
              onFork={() => toggle("dahuang")}
            >
              {openId === "dahuang" && (
                <ForkDrawer displayName="Dahuang" seed="dahuang-copy" maxLength={64} onClose={() => setOpenId(null)} />
              )}
            </SpecimenCard>
          </SectionList>
        </Panel>

        <Panel
          title="The name is checked as you type"
          note={<>
            Fork names become folder names, so the input is monospaced and the rule is enforced
            live: type a space into the open field and the hint line turns into the refusal while
            Fork disables — no round-trip to learn the charset. Skill cards have no preview
            column; the same drawer spans their two-column template.
          </>}
        >
          <SectionList heading="Skills" icon="file">
            <SpecimenCard
              display="api-designer"
              status="synced"
              statusLabel="Up to date here and in Vault."
              files="4"
              size="21.4 KB"
              stamp="Aug 19, 2026 at 14:03"
              stampDetail="Last modified on this Mac · Aug 19, 2026 at 14:03"
              kindPhrase="Codex home skill"
              forkOpen={openSkillId === "api-designer"}
              onFork={() => toggleSkill("api-designer")}
            >
              {openSkillId === "api-designer" && (
                <ForkDrawer displayName="api-designer" seed="api-designer-copy" maxLength={128} onClose={() => setOpenSkillId(null)} />
              )}
            </SpecimenCard>
            <SpecimenCard
              display="code-reviewer"
              status="local-outstanding"
              statusLabel="Newer on this computer. Push to update Vault."
              files="6"
              size="48.2 KB"
              stamp="Aug 21, 2026 at 09:47"
              stampDetail="Last modified on this Mac · Aug 21, 2026 at 09:47"
              kindPhrase="Codex home skill"
              forkOpen={openSkillId === "code-reviewer"}
              onFork={() => toggleSkill("code-reviewer")}
            >
              {openSkillId === "code-reviewer" && (
                <ForkDrawer displayName="code-reviewer" seed="code-reviewer-copy" maxLength={128} onClose={() => setOpenSkillId(null)} />
              )}
            </SpecimenCard>
          </SectionList>
        </Panel>

        <Panel
          title="When the backend refuses"
          note={<>
            A refusal from prepare or apply lands where the hint line was, in the drawer that
            asked — not in the page banner. The backend’s sentence is shown as it arrives; here,
            the taken-name refusal.
          </>}
        >
          <SectionList heading="Skills" icon="file">
            <SpecimenCard
              display="api-designer"
              status="synced"
              statusLabel="Up to date here and in Vault."
              files="4"
              size="21.4 KB"
              stamp="Aug 19, 2026 at 14:03"
              stampDetail="Last modified on this Mac · Aug 19, 2026 at 14:03"
              kindPhrase="Codex home skill"
              forkOpen
            >
              <ForkDrawer
                displayName="api-designer"
                seed="api-designer-copy"
                maxLength={128}
                serverError="the name 'api-designer-copy' is already in use; choose another"
                onClose={() => undefined}
              />
            </SpecimenCard>
          </SectionList>
        </Panel>

        <Panel
          title="What this replaces"
          note={<>
            The shipped form, verbatim markup over the shipped stylesheets. With no grid
            placement it auto-places into the pet template’s <code>preview</code> column and
            inflates that <code>auto</code> column to its own width — the title and meta start at
            mid-card. The label says “Fork local resource”, and the placeholder suggests a name
            the button won’t accept until it is typed again.
          </>}
        >
          <SectionList heading="Pets v2" icon="openai">
            <SpecimenCard
              display="Maple"
              status="local-outstanding"
              statusLabel="Only on this computer. Push to save it in Vault."
              preview={PET_THUMBS.maple}
              petDescription="A border collie with a taste for order."
              files="1"
              size="2.5 MB"
              stamp="Jul 11, 2026 at 00:55"
              stampDetail="Last modified on this Mac · Jul 11, 2026 at 00:55"
              kindPhrase="Codex pet v2"
              forkOpen
            >
              <form className="catalog-inline-form catalog-fork-form" onSubmit={(event) => event.preventDefault()}>
                <label>
                  <span>Fork local resource</span>
                  <input placeholder="Maple-local-copy" readOnly />
                </label>
                <button type="submit" className="btn btn-ghost" disabled>
                  <Icon name="git-fork" size={13} /> Fork
                </button>
                <button type="button" className="btn btn-ghost">Cancel</button>
              </form>
            </SpecimenCard>
          </SectionList>
        </Panel>
      </div>
    </div>
  );
}

// ---------------------------------------------------------------------------
// Styles: the harness shell, then the retired form block the last panel shows.
// ---------------------------------------------------------------------------

const SHELL_CSS = `
/* The app chain is height-locked; a mockup is a document and must scroll. */
html, body { height: auto; min-height: 100%; overflow: visible; }
#root { height: auto; min-height: 100%; overflow: visible; }
.app.mockup-root { height: auto; min-height: 100vh; overflow: visible; }

.mockup-shell { width: 100%; max-width: 1180px; margin: 0 auto; padding: 28px 24px 90px; }
.mockup-head { display: flex; align-items: flex-start; justify-content: space-between; gap: 24px; margin-bottom: 8px; }
.mockup-head h1 { margin: 0 0 6px; color: var(--text-0); font-family: var(--font-pixel); font-size: var(--type-xl); font-weight: 400; }
.mockup-head p { max-width: 78ch; margin: 0; color: var(--text-2); font-size: var(--type-md); line-height: 1.55; }
.mockup-panel { margin-top: 34px; padding-top: 20px; border-top: 1px solid var(--border); }
.mockup-panel > h3 { margin: 0 0 4px; color: var(--text-0); font-family: var(--font-pixel); font-size: var(--type-lg); font-weight: 400; }
.mockup-panel > p { max-width: 82ch; margin: 0 0 14px; color: var(--text-2); font-size: var(--type-sm); line-height: 1.55; }
.mockup-stage { padding: 14px 16px 18px; border: 1px solid var(--border); border-radius: 6px; background: var(--surface-card); }
body { background: var(--app-shell); }
`;

const ARCHIVED_CSS = `
/* The block this redesign retired, kept here verbatim so the "What this
   replaces" panel renders what it replaced. The live drawer styles ship in
   App.css, which this harness imports. */
.catalog-inline-form {
  display: flex;
  align-items: end;
  gap: 8px;
  flex-wrap: wrap;
  padding: 12px;
  border: 1px solid var(--border);
  border-radius: var(--r-sm);
  background: var(--bg-2);
}

.catalog-inline-form label {
  display: grid;
  flex: 1;
  gap: 5px;
  min-width: 210px;
  color: var(--text-1);
  font-size: var(--type-sm);
}
`;

/* ?theme=dark so a headless capture can take the dark panels without clicking. */
const START_DARK = new URLSearchParams(location.search).get("theme") === "dark";
document.documentElement.setAttribute("data-theme", START_DARK ? "dark" : "light");
const style = document.createElement("style");
style.textContent = SHELL_CSS + ARCHIVED_CSS;
document.head.appendChild(style);
const container = document.getElementById("root")! as HTMLElement & { _root?: Root };
container._root ??= createRoot(container);
container._root.render(<StrictMode><Harness /></StrictMode>);
