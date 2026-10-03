import { StrictMode, useState, type ReactNode } from "react";
import { createRoot } from "react-dom/client";
import Icon, { type IconName } from "../src/components/Icons";
import { NoticeZone, type NoticeItem } from "../src/components/notice/Notice";
import { PathBarMenu, type PathBarMenuGroup, type PathBarMenuItem } from "../src/components/sync-review/SyncPathBar";
import "../src/App.css";

/**
 * Mockup: Discard draft on the desktop (PLAN_CLOUD_REPOSITORY_GONE_PUSH_AGAIN,
 * part B). The Owner of a public repository whose draft differs from the
 * public version can discard the draft; nobody else sees the verb.
 *
 * The bar's markup is the shipped bar's, class for class, and the menus are
 * the shipped `PathBarMenu`, held open. The notices are the shipped
 * `NoticeZone`. What is new is the verb, where it sits, and its words.
 *
 * `npm run dev`, then /mockups/cloud-discard-mockup.html
 * (`?verb=all|a|b|c&icon=trash|x|ban|octagon-x&theme=light|dark`).
 */

type Verb = "all" | "a" | "b" | "c";
type VerbIcon = "trash" | "x" | "ban" | "octagon-x";
interface Knobs { verb: Verb; icon: VerbIcon; theme: "light" | "dark" }

function readKnobs(): Knobs {
  const query = new URLSearchParams(window.location.search);
  const pick = <T extends string>(key: string, allowed: readonly T[], fallback: T) => {
    const value = query.get(key) as T | null;
    return value && allowed.includes(value) ? value : fallback;
  };
  return {
    verb: pick("verb", ["all", "a", "b", "c"] as const, "all"),
    icon: pick("icon", ["trash", "x", "ban", "octagon-x"] as const, "x"),
    theme: pick("theme", ["light", "dark"] as const, "light"),
  };
}

const REPOSITORY = "rika/portal";
const PUBLIC_VERSION = 2;
const noop = () => {};

/* ——— the shipped menus' content ————————————————————————————————— */

const discardItem = (icon: IconName): PathBarMenuItem => ({
  id: "discard-managed-cloud-draft",
  label: "Discard draft…",
  icon,
  danger: true,
  title: `Discard draft… · back to public version ${PUBLIC_VERSION}`,
  onSelect: noop,
});

const cloudVerbs: PathBarMenuItem[] = [
  { id: "open-managed-cloud", label: "View draft", detail: REPOSITORY, detailIsPath: true, icon: "external-link", onSelect: noop },
  { id: "view-published-cloud", label: "View published version", icon: "external-link", onSelect: noop },
  { id: "request-managed-cloud-release", label: "Request public release", icon: "upload", onSelect: noop },
];

function projectMenu(cloud: PathBarMenuItem[], tails: PathBarMenuItem[]): PathBarMenuGroup[] {
  return [
    {
      items: [
        { id: "reveal", label: "Reveal in Finder", icon: "folder", onSelect: noop },
        { id: "agent", label: "Agent profile…", detail: "Codex · Claude", icon: "terminal", onSelect: noop },
        ...cloud,
        { id: "rename", label: "Rename project…", icon: "edit", onSelect: noop },
        { id: "delete-project-sessions", label: "Delete all local sessions…", icon: "trash", danger: true, onSelect: noop },
      ],
    },
    {
      label: "Switch to",
      items: [
        { id: "horror", label: "HorrorGame", detail: "~/code/HorrorGame", detailIsPath: true, icon: "git-folder", onSelect: noop },
        { id: "notes", label: "field-notes", detail: "~/notes", detailIsPath: true, icon: "folder", onSelect: noop },
      ],
    },
    { items: [...tails, { id: "add-project", label: "Add project…", icon: "plus", onSelect: noop }] },
  ];
}

/* ——— the shipped bar, held still ————————————————————————————————— */

function Chip({ label, detail, menu }: { label: string; detail: string; menu?: PathBarMenuGroup[] }) {
  if (!menu) {
    return (
      <span className="sync-path-attention sync-path-cloud-status" tabIndex={0} data-tooltip={`Mallard Cloud: ${detail}`}>
        <Icon name="cloud" size={13} />
        <b>{label}</b>
      </span>
    );
  }
  return (
    <span className="spec-anchor">
      <button type="button" className="sync-path-attention sync-path-cloud-status" aria-haspopup="menu" aria-expanded title={`Mallard Cloud: ${detail}`}>
        <Icon name="cloud" size={13} />
        <b>{label}</b>
        <Icon name="chevron-down" size={11} className="sync-path-chevron" />
      </button>
      <PathBarMenu id="spec-chip-menu" groups={menu} label="Mallard Cloud draft" align="end" open registerRef={noop} onClose={noop} />
    </span>
  );
}

function Bar({ menu, chip }: { menu?: PathBarMenuGroup[]; chip: ReactNode }) {
  return (
    <header className="sync-pathbar sync-titlebar">
      <ol className="sync-path" aria-label="portal path">
        <li className="sync-path-item">
          <button type="button" className="sync-path-crumb root">
            <Icon name="storage-mallard" size={14} />
            <span>Mallard Cloud Rika321</span>
            <Icon name="chevron-down" size={11} className="sync-path-chevron" />
          </button>
        </li>
        <li className="sync-path-item">
          <span className="sync-path-separator" aria-hidden="true">/</span>
          <button type="button" className="sync-path-crumb" aria-haspopup="menu" aria-expanded={!!menu}>
            <Icon name="git-folder" size={14} />
            <span>portal</span>
            <Icon name="chevron-down" size={11} className="sync-path-chevron" />
          </button>
          {menu && (
            <PathBarMenu
              id="spec-project-menu"
              groups={menu}
              header={{ label: "portal", detail: "~/code/portal", detailIsPath: true, icon: "git-folder" }}
              label="Project"
              align="start"
              open
              registerRef={noop}
              onClose={noop}
            />
          )}
        </li>
        <li className="sync-path-item leaf-item">
          <span className="sync-path-separator" aria-hidden="true">/</span>
          <span className="sync-path-crumb section leaf" aria-current="page"><span>Git history</span></span>
        </li>
      </ol>
      <div className="sync-path-actions">
        {chip}
        <button type="button" className="sync-path-icon-action" title="Refresh activity" aria-label="Refresh activity">
          <span aria-hidden="true" className="icon-spin-frame"><Icon name="refresh" size={16} /></span>
        </button>
        <div className="sync-path-action-group" role="group" aria-label="Sync actions">
          <button type="button" className="sync-path-sync"><Icon name="download" size={14} /><span className="sync-path-sync-label">Pull</span></button>
          <button type="button" className="sync-path-sync"><Icon name="upload" size={14} /><span className="sync-path-sync-label">Push</span></button>
        </div>
        <span className="sync-path-divider" aria-hidden="true" />
        <div className="sync-path-overflow">
          <button type="button" className="sync-path-more" title="App actions" aria-label="App actions"><Icon name="more" size={16} /></button>
        </div>
      </div>
    </header>
  );
}

const DRAFT_CHIP = { label: "Not requested", detail: `Public version ${PUBLIC_VERSION}. Draft version 5. Unpublished draft. The current draft is not under review.` };
const PUBLIC_CHIP = { label: "Published", detail: `Public version ${PUBLIC_VERSION}. Draft version 2. Draft matches published version. The draft matches the published version.` };

function Body({ children }: { children?: ReactNode }) {
  return <main className="sync-main">{children ?? <p className="spec-hint">Git history · 14 sessions</p>}</main>;
}

/* ——— the frames ——————————————————————————————————————————————— */

function Frame({ title, note, pick, tall, children }: { title: string; note: string; pick?: boolean; tall?: boolean; children: ReactNode }) {
  return (
    <div>
      <h3 className="spec-col-title">{title}{pick && <em>recommended</em>}</h3>
      <p className="spec-col-note">{note}</p>
      <div className={`spec-frame ${tall ? "tall" : "short"}`}>{children}</div>
    </div>
  );
}

const notice = (item: Omit<NoticeItem, "key" | "severity">): NoticeItem[] => [{ key: "cloud-draft", severity: "warning", ...item }];

function After({ items, chip }: { items: NoticeItem[]; chip: { label: string; detail: string } }) {
  return (
    <>
      <Bar chip={<Chip {...chip} />} />
      <Body>
        <NoticeZone items={items} subject="notes about this project activity" className="sync-history-notices" timing="standing" label="Notes about this project activity" />
        <p className="spec-hint" style={{ marginTop: 10 }}>Git history · 14 sessions</p>
      </Body>
    </>
  );
}

function Specimen() {
  const [knobs, setKnobs] = useState<Knobs>(readKnobs);
  const set = <K extends keyof Knobs>(key: K, value: Knobs[K]) => {
    const next = { ...knobs, [key]: value };
    setKnobs(next);
    const query = new URLSearchParams({ verb: next.verb, icon: next.icon, theme: next.theme });
    window.history.replaceState(null, "", `${window.location.pathname}?${query}`);
    document.documentElement.dataset.theme = next.theme;
  };
  const seg = <K extends keyof Knobs>(key: K, value: Knobs[K], label: string) => (
    <button key={String(value)} type="button" className="spec-seg" aria-pressed={knobs[key] === value} onClick={() => set(key, value)}>{label}</button>
  );
  const shows = (verb: Exclude<Verb, "all">) => knobs.verb === "all" || knobs.verb === verb;
  const discard = discardItem(knobs.icon);
  const message = `Discard the draft of “${REPOSITORY}”?\n\nRemoves every push after public version ${PUBLIC_VERSION} from Mallard Cloud. The draft returns to the public version.\n\nNothing is deleted on this computer. This cannot be undone.`;

  return (
    <div className="spec">
      <h1 className="spec-title">Discard draft · desktop</h1>
      <p className="spec-sub">
        The bar, the menus and the notices are the shipped components. Shown for the Owner of the public repository {REPOSITORY},
        public version {PUBLIC_VERSION}, with a newer draft.
      </p>

      <div className="spec-controls">
        <div className="spec-group"><span>Verb</span>{seg("verb", "all", "All")}{seg("verb", "a", "A")}{seg("verb", "b", "B")}{seg("verb", "c", "C")}</div>
        <div className="spec-group"><span>Icon</span>{seg("icon", "x", "x")}{seg("icon", "trash", "trash")}{seg("icon", "ban", "ban")}{seg("icon", "octagon-x", "octagon-x")}</div>
        <div className="spec-group"><span>Theme</span>{seg("theme", "light", "Light")}{seg("theme", "dark", "Dark")}</div>
      </div>

      <h2 className="spec-h2">1 · Where the verb lives</h2>
      <p className="spec-lede">Today the draft's verbs are icon pills in the project menu's footer: View draft, View published version, Request public release.</p>
      <div className="spec-row">
        <Frame title="Today" note="No way to discard. Seven icon pills; each names itself in a tooltip." tall>
          <Bar menu={projectMenu(cloudVerbs, [])} chip={<Chip {...DRAFT_CHIP} />} />
          <Body />
        </Frame>
        {shows("a") && (
          <Frame title="A · One more icon pill" note="Beside the other draft verbs, in danger ink, as the menu draws every verb today. Eight pills, and two of them in red: this one and Delete all local sessions. The x stands in for a mark of its own." pick tall>
            <Bar menu={projectMenu([...cloudVerbs, discard], [])} chip={<Chip {...DRAFT_CHIP} />} />
            <Body />
          </Frame>
        )}
        {shows("b") && (
          <Frame title="B · The Cloud chip opens the draft's menu" note="The chip already says where the draft stands. It becomes a button, a chevron wider, and the four draft verbs move to it as rows with their names. The project menu goes back to four pills. It moves three verbs that shipped in the project menu." tall>
            <Bar
              menu={projectMenu([], [])}
              chip={<Chip {...DRAFT_CHIP} menu={[{ items: cloudVerbs }, { items: [discard] }]} />}
            />
            <Body />
          </Frame>
        )}
        {shows("c") && (
          <Frame title="C · A named pill in the footer" note="The verb keeps its words, beside Add project. The footer's right side is where the menu puts what adds; a verb that removes sits oddly there." tall>
            <Bar menu={projectMenu(cloudVerbs, [discard])} chip={<Chip {...DRAFT_CHIP} />} />
            <Body />
          </Frame>
        )}
      </div>

      <h2 className="spec-h2">2 · Asking first</h2>
      <p className="spec-lede">Every destructive verb in the app asks through the system's dialog today: fourteen of them, none in a dialog of the app's own.</p>
      <div className="spec-row" style={{ gridTemplateColumns: "repeat(auto-fit, minmax(520px, 1fr))" }}>
        <Frame title="1 · The system's dialog" note="As Delete all local sessions and Clear storage data ask. Drawn here; the system styles it." pick>
          <Bar chip={<Chip {...DRAFT_CHIP} />} />
          <div className="spec-scrim">
            <div className="spec-alert" role="img" aria-label="The system's confirmation dialog">
              <div className="spec-alert-mark">!</div>
              <h3>Discard draft?</h3>
              <p>{message}</p>
              <div className="spec-alert-buttons"><span>Cancel</span><span className="default">Discard</span></div>
            </div>
          </div>
        </Frame>
        <Frame title="2 · The app's own dialog" note="The modal the choosers use. It would be the app's first confirmation of its own, and needs a danger button the app does not have.">
          <Bar chip={<Chip {...DRAFT_CHIP} />} />
          <div className="spec-scrim">
            {/* Open, not modal: the specimen shows it in its frame. */}
            <dialog open className="sync-modal sync-discard-dialog" aria-label="Discard draft">
              <header className="sync-modal-header">
                <div>
                  <span className="sync-eyebrow">{REPOSITORY}</span>
                  <h1>Discard draft</h1>
                </div>
                <button type="button" className="btn btn-ghost" aria-label="Close"><Icon name="x" size={17} /></button>
              </header>
              <div className="sync-modal-body">
                <p>Removes every push after public version {PUBLIC_VERSION} from Mallard Cloud. The draft returns to the public version.</p>
                <p>Nothing is deleted on this computer. This cannot be undone.</p>
              </div>
              <footer className="sync-modal-footer">
                <div>
                  <button type="button" className="btn">Cancel</button>
                  <button type="button" className="btn danger">Discard</button>
                </div>
              </footer>
            </dialog>
          </div>
        </Frame>
      </div>

      <h2 className="spec-h2">3 · Afterwards</h2>
      <p className="spec-lede">One note under the bar, on a computer that was on the discarded pushes. It ends by itself after the Pull.</p>
      <div className="spec-row">
        <Frame title="Back at the public version" note="On this computer and on every other that had pushed or pulled the draft. The chip reads Published.">
          <After chip={PUBLIC_CHIP} items={notice({ title: "Draft discarded", message: `— Mallard Cloud holds public version ${PUBLIC_VERSION}. Pull, then push again.` })} />
        </Frame>
        <Frame title="Another computer has pushed since" note="">
          <After chip={DRAFT_CHIP} items={notice({ title: "Draft discarded and pushed again", message: "— Pull, then push again." })} />
        </Frame>
        <Frame title="A private repository, discarded on the website" note="Nothing is left to compare with; Push starts a new draft.">
          <After chip={{ label: "Not pushed", detail: "Private. No draft yet. Registered on Mallard Cloud; nothing has been pushed yet." }} items={notice({ title: "Draft discarded", message: "— Push creates a new draft." })} />
        </Frame>
      </div>

      <h2 className="spec-h2">4 · Who sees the verb</h2>
      <div className="spec-panel">
        <table>
          <thead><tr><th>Repository</th><th>Owner</th><th>Admin</th><th>Member, contributor</th></tr></thead>
          <tbody>
            <tr><td>Public, draft differs from the public version</td><td><b>Discard draft…</b></td><td>website only</td><td>no</td></tr>
            <tr><td>Public, draft is the public version</td><td>nothing to discard</td><td>nothing to discard</td><td>no</td></tr>
            <tr><td>Private, also one made private after a release</td><td>website only</td><td>website only</td><td>no</td></tr>
          </tbody>
        </table>
      </div>
    </div>
  );
}

document.documentElement.dataset.theme = readKnobs().theme;
createRoot(document.getElementById("root")!).render(<StrictMode><Specimen /></StrictMode>);
