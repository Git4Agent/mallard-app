import { StrictMode, useEffect, useState, type ReactNode } from "react";
import { createRoot } from "react-dom/client";
import Icon from "../src/components/Icons";
import SyncPathBar, { type PathBarCrumb, type PathBarMenuGroup } from "../src/components/sync-review/SyncPathBar";
import { StorageLockedForm } from "../src/components/project-sync/StorageSettings";
import { writeClipboardText } from "../src/components/sync-review/clipboard";
import { formatRelativeTime } from "../src/clock";
import type { StorageConnectionHealth, StorageProviderDescriptor, StorageSummary } from "../src/types";
import "../src/App.css";

/**
 * Mockup: the settled Mallard Cloud storage's page. Today the service
 * address sits in a read-only input beside Account, in the grey every
 * locked setting wears: nothing opens it, and copying it means selecting
 * the text by hand. Three directions make it the visible, copyable,
 * openable fact on the page. Other providers' pages are not touched.
 *
 *   Now  the shipped StorageLockedForm, unchanged, for comparison.
 *   A    Link field      Service full width at the top, with the token
 *                        field's in-field verbs: Copy | Open.
 *   B    Address banner  the address leads the card in display type, with
 *                        Copy and Open website buttons; the account rides
 *                        under it and the token keeps its field below.
 *   C    Facts sheet     label · value · verbs rows instead of input boxes.
 *
 * Static data with generic names. `npm run dev`, then
 * /mockups/cloud-storage-mockup.html (`?dir=all|now|a|b|c&conn=configured|verified|refused&origin=production|dev&repos=now|web&width=wide|narrow&theme=light|dark`).
 */

/* ——— knobs ———————————————————————————————————————————————————————— */

type Dir = "now" | "a" | "b" | "c";
type Conn = "configured" | "verified" | "refused";
type Origin = "production" | "dev";
type Repos = "now" | "web";
type Width = "wide" | "narrow";
type Theme = "light" | "dark";
interface Knobs { dir: Dir | "all"; conn: Conn; origin: Origin; repos: Repos; width: Width; theme: Theme }

function readKnobs(): Knobs {
  const query = new URLSearchParams(window.location.search);
  const pick = <T extends string>(key: string, allowed: readonly T[], fallback: T): T => {
    const value = query.get(key);
    return allowed.find((candidate) => candidate === value) ?? fallback;
  };
  return {
    dir: pick("dir", ["all", "now", "a", "b", "c"] as const, "all"),
    conn: pick("conn", ["configured", "verified", "refused"] as const, "configured"),
    origin: pick("origin", ["production", "dev"] as const, "production"),
    repos: pick("repos", ["now", "web"] as const, "now"),
    width: pick("width", ["wide", "narrow"] as const, "wide"),
    theme: pick("theme", ["light", "dark"] as const, "light"),
  };
}

/* ——— data ————————————————————————————————————————————————————————— */

const NAME = "Mallard Cloud Rika321";
const LOGIN = "Rika321";
const ORIGINS: Record<Origin, string> = { production: "https://mallard-ai.com", dev: "http://localhost:8787" };
const WIDTHS: Record<Width, number> = { wide: 980, narrow: 700 };
const TOKEN = "desktop-session-demo-0123456789abcdef";
const MASK = "••••••••••••••••••••••••";
/* The shipped help says "Reconnect through the lane", an internal word. */
const TOKEN_HELP = "Issued at sign-in and valid 90 days. Reconnect to renew it.";

const DESCRIPTOR: StorageProviderDescriptor = {
  key: "managed_cloud",
  config_version: 1,
  display_name: "Mallard Cloud",
  description: "Private snapshots with team sharing and web access.",
  default_name: "Mallard Cloud",
  display_rank: 5,
  icon: "cloud",
  clear_mode: "unavailable",
  fields: [],
  secrets: [{
    secret_id: "account_token",
    label: "Sign-in token",
    required: true,
    placeholder: "Issued by the browser sign-in",
    help: "The desktop session Mallard Cloud issued at sign-in, valid 90 days. Reconnect through the lane to renew it.",
  }],
  managed_secret_id: "account_token",
  local_setup: "managed_cloud",
};

const REFUSED: StorageConnectionHealth = {
  state: "token_expired",
  code: "managed_cloud_reconnect_required",
  observed_at: 1,
  message: "Mallard Cloud no longer accepts this computer’s sign-in. Reconnect to sign in again.",
};

function storageOf(knobs: Knobs): StorageSummary {
  const origin = ORIGINS[knobs.origin];
  return {
    storage_id: "storage-mallard-cloud",
    name: NAME,
    provider: "managed_cloud",
    config_version: 1,
    provider_label: "Mallard Cloud",
    icon: "cloud",
    available: true,
    summary: [
      { field_id: "account_login", label: "Account", value: LOGIN, is_path: false },
      { field_id: "service_origin", label: "Service", value: origin, is_path: false },
    ],
    settings: { service_origin: origin, account_id: "user_1", account_login: LOGIN },
    secrets: [{ secret_id: "account_token", present: true }],
    connection: knobs.conn === "refused" ? REFUSED : null,
  };
}

interface Repo { id: string; name: string; slug: string; git: boolean; revision: number; ago: number }
const HOUR = 3_600;
const REPOS: Repo[] = [
  { id: "b1", name: "physics 225", slug: "physics-225", git: false, revision: 5, ago: HOUR },
  { id: "b2", name: "thesis-notes", slug: "thesis-notes", git: true, revision: 12, ago: 26 * HOUR },
  { id: "b3", name: "lab-scripts", slug: "lab-scripts", git: true, revision: 3, ago: 9 * 24 * HOUR },
];

const nowSeconds = () => Math.floor(Date.now() / 1000);

/** `https://` and `mallard-ai.com`, so the scheme can recede. */
function splitOrigin(origin: string): [string, string] {
  const at = origin.indexOf("://");
  return at < 0 ? ["", origin] : [origin.slice(0, at + 3), origin.slice(at + 3)];
}

/* The mock has no webview opener: Open says what it would do. */
const say = (text: string) => window.dispatchEvent(new CustomEvent<string>("spec-open", { detail: text }));
const openUrl = (url: string, what: string) => say(`Opens ${what} in your browser · ${url}`);

/* ——— shared pieces ——————————————————————————————————————————————— */

const COPIED_FOR_MS = 2_000;

/** The Cloud panel's copy receipt: Copied for two seconds, on one key. */
function useCopied() {
  const [copied, setCopied] = useState<string | null>(null);
  useEffect(() => {
    if (!copied) return;
    const timer = window.setTimeout(() => setCopied(null), COPIED_FOR_MS);
    return () => window.clearTimeout(timer);
  }, [copied]);
  const copy = async (key: string, value: string) => {
    try {
      await writeClipboardText(value);
    } catch {
      /* Some browsers refuse a page's clipboard; the app's webview does not.
         The receipt still shows, so the state can be judged anywhere. */
      say("This browser blocked the clipboard — the app’s webview allows it");
    }
    setCopied(key);
  };
  return { copied, copy };
}

/** The shipped reveal: Show for 30 s, with a countdown. */
function useReveal() {
  const [hidesAt, setHidesAt] = useState<number | null>(null);
  const [now, setNow] = useState(() => Date.now());
  useEffect(() => {
    if (hidesAt === null) return;
    const tick = window.setInterval(() => {
      const at = Date.now();
      setNow(at);
      if (at >= hidesAt) setHidesAt(null);
    }, 1000);
    return () => window.clearInterval(tick);
  }, [hidesAt]);
  const shown = hidesAt !== null;
  const secondsLeft = hidesAt !== null ? Math.max(0, Math.ceil((hidesAt - now) / 1000)) : 0;
  const toggle = () => {
    setNow(Date.now());
    setHidesAt(shown ? null : Date.now() + 30_000);
  };
  return { shown, secondsLeft, toggle };
}

const Lock = () => <span className="sync-storage-lock" title="Fixed for this storage"><Icon name="lock" size={11} /></span>;

function CardHeader() {
  const [name, setName] = useState(NAME);
  return (
    <header>
      <span className="sync-storage-icon"><Icon name="storage-mallard" size={18} /></span>
      <form className="sync-storage-name" onSubmit={(event) => event.preventDefault()}>
        <input value={name} onChange={(event) => setName(event.target.value)} aria-label="Storage name" placeholder="Storage name" />
      </form>
      <span className="sync-storage-editor-provider">Mallard Cloud</span>
    </header>
  );
}

function GuideRow({ knobs, renews }: { knobs: Knobs; renews: boolean }) {
  const suspended = knobs.conn === "refused";
  const verified = knobs.conn === "verified";
  const tail = renews ? " Access renews automatically." : "";
  const copy = verified
    ? `Connected · verified ${formatRelativeTime(nowSeconds() - 120)}.${tail}`
    : `Configured. Test the connection to verify it.${tail}`;
  return (
    <div className="storage-setup-guide" role={suspended ? "alert" : "note"}>
      <Icon name={suspended ? "alert-triangle" : verified ? "check-circle" : "info"} size={13} />
      <span className={suspended ? "storage-setup-guide-error" : "storage-setup-guide-copy"}>{suspended ? REFUSED.message : copy}</span>
      {suspended && (
        <button type="button" className="btn storage-setup-guide-button storage-setup-agent-cta">
          <Icon name="refresh" size={12} /> Reconnect…
        </button>
      )}
    </div>
  );
}

function CardFooter({ knobs }: { knobs: Knobs }) {
  return (
    <footer className="sync-storage-editor-footer">
      <span className="sync-storage-fixed">
        <Icon name="lock" size={12} />
        This storage’s destination and credentials are fixed. To sync somewhere else, add a new storage.
      </span>
      {knobs.conn === "verified" && (
        <span className="storage-setup-verified" role="status"><Icon name="check-circle" size={13} /> Connection verified</span>
      )}
      <button type="button" className="btn" disabled={knobs.conn === "refused"}>Test connection</button>
    </footer>
  );
}

/** The shipped secret field, with plain help. */
function TokenField({ knobs, id }: { knobs: Knobs; id: string }) {
  const { copied, copy } = useCopied();
  const reveal = useReveal();
  return (
    <div className="storage-provider-field">
      <span className="form-label-row">
        <label className="sync-field-name" htmlFor={id}>Sign-in token</label>
        {knobs.conn === "refused" && <span className="sync-storage-refused-chip"><Icon name="alert-triangle" size={10} /> Refused</span>}
        <Lock />
        {copied
          ? <span className="sync-secret-cap copied" role="status"><Icon name="check" size={11} /> Copied</span>
          : reveal.shown
            ? <span className="sync-secret-cap"><Icon name="clock" size={11} /> Hides in {reveal.secondsLeft} s</span>
            : null}
      </span>
      <span className="sync-secret-input has-copy">
        <input type={reveal.shown ? "text" : "password"} id={id} value={reveal.shown ? TOKEN : MASK} readOnly spellCheck={false} autoComplete="off" />
        <span className="sync-secret-verbs">
          <button type="button" className="sync-secret-copy" title="Copy Sign-in token" aria-label="Copy Sign-in token" onClick={() => void copy("token", TOKEN)}>
            <Icon name="copy" size={13} />
          </button>
          <button type="button" className="sync-secret-reveal" aria-pressed={reveal.shown} onClick={reveal.toggle}>
            {reveal.shown ? "Hide" : "Show"}
          </button>
        </span>
      </span>
      <small className="sync-storage-field-help">{TOKEN_HELP}</small>
    </div>
  );
}

/* ——— Now · the shipped card ———————————————————————————————————————— */

function NowCard({ knobs }: { knobs: Knobs }) {
  const storage = storageOf(knobs);
  const [name, setName] = useState(storage.name);
  return (
    <StorageLockedForm
      storage={{ ...storage, name }}
      descriptor={DESCRIPTOR}
      disabled={false}
      onRename={async (next) => { setName(next); return true; }}
      onRevealSecret={async () => TOKEN}
      onTest={() => undefined}
      testing={false}
      verifiedAt={knobs.conn === "verified" ? Date.now() - 120_000 : null}
      reconnect={{ open: false, onOpen: () => undefined, band: null }}
    />
  );
}

/* ——— A · link field ————————————————————————————————————————————————— */

function DirectionA({ knobs }: { knobs: Knobs }) {
  const origin = ORIGINS[knobs.origin];
  const { copied, copy } = useCopied();
  const done = copied === "service";
  return (
    <article className="sync-storage-card sync-storage-locked">
      <CardHeader />
      <GuideRow knobs={knobs} renews />
      <div className="sync-storage-fields storage-provider-fields" aria-label="Mallard Cloud connection details">
        <div className="storage-provider-field storage-provider-url-field">
          <span className="form-label-row">
            <label className="sync-field-name" htmlFor="a-service">Service</label>
            <Lock />
            {done && <span className="sync-secret-cap copied" role="status"><Icon name="check" size={11} /> Copied</span>}
          </span>
          <span className="csm-link-input">
            <span className="csm-link-mark"><Icon name="link" size={14} /></span>
            <input type="url" id="a-service" value={origin} readOnly spellCheck={false} />
            <span className="sync-secret-verbs">
              <button
                type="button"
                className={`sync-secret-copy${done ? " copied" : ""}`}
                title="Copy address"
                aria-label={`Copy ${origin}`}
                onClick={() => void copy("service", origin)}
              >
                <Icon name={done ? "check" : "copy"} size={13} />
              </button>
              <button type="button" className="sync-secret-reveal" title={`Open ${origin} in your browser`} onClick={() => openUrl(origin, "Mallard Cloud")}>
                Open <Icon name="external-link" size={12} />
              </button>
            </span>
          </span>
          <small className="sync-storage-field-help">Your repositories and sharing, on the web.</small>
        </div>
        <div className="storage-provider-field storage-provider-fact">
          <span className="form-label-row"><label className="sync-field-name" htmlFor="a-account">Account</label></span>
          <input type="text" id="a-account" value={LOGIN} readOnly spellCheck={false} />
        </div>
        <TokenField knobs={knobs} id="a-token" />
      </div>
      <CardFooter knobs={knobs} />
    </article>
  );
}

/* ——— B · address banner —————————————————————————————————————————————— */

function DirectionB({ knobs }: { knobs: Knobs }) {
  const origin = ORIGINS[knobs.origin];
  const [scheme, host] = splitOrigin(origin);
  const { copied, copy } = useCopied();
  const done = copied === "service";
  return (
    <article className="sync-storage-card sync-storage-locked">
      <CardHeader />
      <GuideRow knobs={knobs} renews />
      <section className="csm-service" aria-label="Mallard Cloud address">
        <div className="csm-address">
          <span className="csm-address-mark"><Icon name="link" size={16} /></span>
          <button type="button" className="csm-address-url" title={`Open ${origin} in your browser`} onClick={() => openUrl(origin, "Mallard Cloud")}>
            <span className="csm-scheme">{scheme}</span><span className="csm-host">{host}</span>
          </button>
          <span className="csm-address-verbs">
            <button
              type="button"
              className={`btn${done ? " storage-setup-copied-flash" : ""}`}
              aria-label={done ? "Address copied" : `Copy ${origin}`}
              onClick={() => void copy("service", origin)}
            >
              <Icon name={done ? "check" : "copy"} size={13} /> {done ? "Copied" : "Copy"}
            </button>
            <button type="button" className="btn" onClick={() => openUrl(origin, "Mallard Cloud")}>
              <Icon name="external-link" size={13} /> Open website
            </button>
          </span>
        </div>
        <div className="csm-identity">
          <span><Icon name="user" size={12} /> Signed in as <b>{LOGIN}</b></span>
        </div>
      </section>
      <div className="sync-storage-fields storage-provider-fields csm-b-fields" aria-label="Mallard Cloud credential">
        <TokenField knobs={knobs} id="b-token" />
      </div>
      <CardFooter knobs={knobs} />
    </article>
  );
}

/* ——— C · facts sheet ————————————————————————————————————————————————— */

function DirectionC({ knobs }: { knobs: Knobs }) {
  const origin = ORIGINS[knobs.origin];
  const [scheme, host] = splitOrigin(origin);
  const { copied, copy } = useCopied();
  const reveal = useReveal();
  const copyVerb = (key: string, value: string, what: string) => (
    <button
      type="button"
      className={`csm-icon-verb${copied === key ? " copied" : ""}`}
      title={`Copy ${what}`}
      aria-label={copied === key ? `${what} copied` : `Copy ${what}`}
      onClick={() => void copy(key, value)}
    >
      <Icon name={copied === key ? "check" : "copy"} size={13} />{copied === key && " Copied"}
    </button>
  );
  return (
    <article className="sync-storage-card sync-storage-locked">
      <CardHeader />
      <GuideRow knobs={knobs} renews />
      <dl className="csm-sheet" aria-label="Mallard Cloud connection details">
        <div className="csm-sheet-row">
          <dt>Service</dt>
          <dd className="csm-sheet-value">
            <button type="button" className="csm-sheet-link" title={`Open ${origin} in your browser`} onClick={() => openUrl(origin, "Mallard Cloud")}>
              <Icon name="link" size={14} />
              <span><span className="csm-scheme">{scheme}</span><span className="csm-host">{host}</span></span>
            </button>
          </dd>
          <dd className="csm-sheet-verbs">
            {copyVerb("service", origin, "address")}
            <button type="button" className="csm-icon-verb" title={`Open ${origin} in your browser`} onClick={() => openUrl(origin, "Mallard Cloud")}>
              <Icon name="external-link" size={13} /> Open
            </button>
          </dd>
        </div>
        <div className="csm-sheet-row">
          <dt>Account</dt>
          <dd className="csm-sheet-value"><Icon name="user" size={14} /> {LOGIN}</dd>
          <dd className="csm-sheet-verbs">{copyVerb("account", LOGIN, "account name")}</dd>
        </div>
        <div className="csm-sheet-row">
          <dt>
            Sign-in token
            {knobs.conn === "refused" && <span className="sync-storage-refused-chip"><Icon name="alert-triangle" size={10} /> Refused</span>}
          </dt>
          <dd className="csm-sheet-value">
            <span className="csm-sheet-mask">{reveal.shown ? TOKEN : MASK}</span>
            <small>{reveal.shown ? `Hides in ${reveal.secondsLeft} s` : "Valid 90 days"}</small>
          </dd>
          <dd className="csm-sheet-verbs">
            {copyVerb("token", TOKEN, "sign-in token")}
            <button type="button" className="csm-icon-verb" aria-pressed={reveal.shown} onClick={reveal.toggle}>
              {reveal.shown ? "Hide" : "Show"}
            </button>
          </dd>
        </div>
      </dl>
      <CardFooter knobs={knobs} />
    </article>
  );
}

/* ——— the page around the card ————————————————————————————————————————— */

function PathBar({ knobs, redesigned }: { knobs: Knobs; redesigned: boolean }) {
  const origin = ORIGINS[knobs.origin];
  const [, host] = splitOrigin(origin);
  const crumbs: PathBarCrumb[] = [
    { id: "storages", label: "Storages", icon: "storage", tone: "root", title: "All storages", ariaLabel: "All storages", onActivate: () => undefined },
    {
      id: "storage",
      label: NAME,
      title: `${NAME} · ${LOGIN}`,
      ariaLabel: `${NAME}. Storage menu`,
      menuHeader: { label: NAME, detail: `${LOGIN} · ${host}`, icon: "storage-mallard" },
      menu: [{
        label: "Switch to",
        items: [
          { id: "r2", label: "mallard-quick", detail: "Cloudflare R2", icon: "storage-r2", onSelect: () => undefined },
          { id: "local", label: "Backup disk", detail: "/Volumes/backup/mallard-storage", detailIsPath: true, icon: "storage-local", onSelect: () => undefined },
        ],
      }],
    },
  ];
  /* The redesign adds the local folder's "Reveal in Finder" counterpart. */
  const overflow: PathBarMenuGroup[] = [{
    label: "Storage",
    items: [
      ...(redesigned ? [{
        id: "open-website",
        label: "Open website",
        detail: host,
        icon: "external-link" as const,
        onSelect: () => openUrl(origin, "Mallard Cloud"),
      }] : []),
      { id: "rename", label: "Rename storage…", icon: "edit", onSelect: () => undefined },
      {
        id: "remove",
        label: "Remove storage…",
        icon: "x",
        danger: true,
        disabled: true,
        disabledReason: "Linked to 1 project — unlink first",
        onSelect: () => undefined,
      },
    ],
  }];
  return (
    <SyncPathBar
      crumbs={crumbs}
      overflow={overflow}
      overflowLabel="Storage and app actions"
      ariaLabel={`${NAME} path`}
      actions={(
        <button type="button" className="sync-path-sync" title={`Refresh repositories in ${NAME}`} aria-label={`Refresh repositories in ${NAME}`}>
          <Icon name="refresh" size={14} />
          <span className="sync-path-sync-label">Refresh</span>
        </button>
      )}
    />
  );
}

function RepoRow({ repo, web, host }: { repo: Repo; web: string | null; host: string }) {
  const [open, setOpen] = useState(false);
  return (
    <div className="sync-storage-repository-item">
      <div className={`sync-storage-repository-row${web ? " csm-repo-web" : ""}`}>
        <span className="sync-storage-repository-icon"><Icon name={repo.git ? "git-folder" : "folder"} size={14} /></span>
        <strong className="sync-storage-repository-name">{repo.name}</strong>
        <span className="sync-storage-repository-chips">
          <span className="sync-storage-repo-chip"><span>Version</span><b>{repo.revision}</b></span>
          <span className="sync-storage-repo-chip"><span>Updated</span><b>{formatRelativeTime(nowSeconds() - repo.ago)}</b></span>
        </span>
        {web && (
          <button
            type="button"
            className="sync-history-commit-toggle"
            title={`Open ${repo.name} on ${host}`}
            aria-label={`Open ${repo.name} on ${host}`}
            onClick={() => openUrl(web, `${repo.name} on ${host}`)}
          >
            <Icon name="external-link" size={12} />
          </button>
        )}
        <button
          type="button"
          className="sync-history-commit-toggle"
          aria-expanded={open}
          aria-label={`${open ? "Hide" : "Show"} details for ${repo.name}`}
          title={`${open ? "Hide" : "Show"} details for ${repo.name}`}
          onClick={() => setOpen((value) => !value)}
        >
          <Icon name={open ? "chevron-up" : "chevron-down"} size={12} />
        </button>
      </div>
      {open && (
        <dl className="sync-storage-repository-details">
          <dt>Kind</dt>
          <dd>{repo.git ? "Git project" : "Project"}</dd>
          <dt>Updated</dt>
          <dd>{new Date((nowSeconds() - repo.ago) * 1000).toLocaleString()}</dd>
        </dl>
      )}
    </div>
  );
}

function Page({ knobs, dir }: { knobs: Knobs; dir: Dir }) {
  const origin = ORIGINS[knobs.origin];
  const [, host] = splitOrigin(origin);
  const web = dir !== "now" && knobs.repos === "web";
  return (
    <>
      <PathBar knobs={knobs} redesigned={dir !== "now"} />
      <main className="sync-main sync-project-links-page sync-storage-settings-page">
        <section className="profile-links-section" aria-label={NAME}>
          <div className="sync-storage-settings-below sync-storage-settings-dedicated">
            <div className="sync-storage-details-panel">
              {dir === "now" ? <NowCard knobs={knobs} />
                : dir === "a" ? <DirectionA knobs={knobs} />
                  : dir === "b" ? <DirectionB knobs={knobs} />
                    : <DirectionC knobs={knobs} />}
            </div>
            <section className="sync-storage-repositories" aria-labelledby={`repos-${dir}`}>
              <div className="sync-storage-repositories-heading">
                <h2 id={`repos-${dir}`}>Repositories</h2>
              </div>
              <div className="sync-storage-repository-list">
                {REPOS.map((repo) => (
                  <RepoRow key={repo.id} repo={repo} host={host} web={web ? `${origin}/${LOGIN}/${repo.slug}` : null} />
                ))}
              </div>
            </section>
          </div>
        </section>
      </main>
    </>
  );
}

/* ——— specimen ————————————————————————————————————————————————————————— */

const DIRECTIONS: { id: Dir; title: string; note: ReactNode }[] = [
  {
    id: "now",
    title: "Now · the shipped card",
    note: "Service is a read-only input beside Account, in the grey every locked setting wears. Nothing opens it, and copying means selecting the text by hand.",
  },
  {
    id: "a",
    title: "A · Link field — the smallest change",
    note: "Service moves first and spans the card, with the token field’s own in-field verbs: Copy | Open. The value prints in page ink beside a link mark, so it stops reading as a greyed setting. Account and the token share the row below.",
  },
  {
    id: "b",
    title: "B · Address banner — the address leads (chosen)",
    note: "The address is the card’s first fact: display type in a box of its own, with Copy and Open website as labelled buttons, and the host itself clickable. The account becomes one quiet line under it; the token keeps its field below. Renewal stays in the guide row, where Dropbox says it too.",
  },
  {
    id: "c",
    title: "C · Facts sheet — one row per fact",
    note: "Input boxes give way to label · value · verbs rows. The address is a link with Copy and Open, the account copies, the token keeps Copy and Show. The lightest card; the address is clear but shares the stage.",
  },
];

function Toast() {
  const [text, setText] = useState<string | null>(null);
  useEffect(() => {
    let timer = 0;
    const show = (event: Event) => {
      setText((event as CustomEvent<string>).detail);
      window.clearTimeout(timer);
      timer = window.setTimeout(() => setText(null), 2_600);
    };
    window.addEventListener("spec-open", show);
    return () => {
      window.removeEventListener("spec-open", show);
      window.clearTimeout(timer);
    };
  }, []);
  return text ? <div className="spec-toast" role="status">{text}</div> : null;
}

function Specimen() {
  const [knobs, setKnobs] = useState<Knobs>(readKnobs);
  const set = <K extends keyof Knobs>(key: K, value: Knobs[K]) => {
    const next = { ...knobs, [key]: value };
    setKnobs(next);
    const query = new URLSearchParams({
      dir: next.dir, conn: next.conn, origin: next.origin, repos: next.repos, width: next.width, theme: next.theme,
    });
    window.history.replaceState(null, "", `${window.location.pathname}?${query}`);
    document.documentElement.dataset.theme = next.theme;
  };
  const seg = <K extends keyof Knobs>(key: K, value: Knobs[K], label: string) => (
    <button key={String(value)} type="button" className="spec-seg" aria-pressed={knobs[key] === value} onClick={() => set(key, value)}>{label}</button>
  );
  const shown = DIRECTIONS.filter((direction) => knobs.dir === "all" || knobs.dir === direction.id);
  const frameKey = `${knobs.conn}-${knobs.origin}`;
  return (
    <div className="spec">
      <h1 className="spec-title">Mallard Cloud storage page — directions</h1>
      <p className="spec-sub">
        The service address should be the page’s visible, copyable, openable fact. Every frame is the whole page and
        follows the same knobs; Copy writes the real clipboard, Open says where it would go. Only the Mallard Cloud
        storage changes — other providers keep today’s card.
      </p>

      <div className="spec-controls">
        <div className="spec-group"><span>Show</span>{seg("dir", "all", "All")}{seg("dir", "now", "Now")}{seg("dir", "a", "A")}{seg("dir", "b", "B")}{seg("dir", "c", "C")}</div>
        <div className="spec-group"><span>Connection</span>{seg("conn", "configured", "Untested")}{seg("conn", "verified", "Verified")}{seg("conn", "refused", "Refused")}</div>
        <div className="spec-group"><span>Service</span>{seg("origin", "production", "mallard-ai.com")}{seg("origin", "dev", "Dev server")}</div>
        <div className="spec-group"><span>Repository rows</span>{seg("repos", "now", "As now")}{seg("repos", "web", "+ Website link")}</div>
        <div className="spec-group"><span>Width</span>{seg("width", "wide", "Wide")}{seg("width", "narrow", "Narrow")}</div>
        <div className="spec-group"><span>Theme</span>{seg("theme", "light", "Light")}{seg("theme", "dark", "Dark")}</div>
      </div>

      <div className="spec-panel spec-pick">
        <strong>Chosen: B, the address banner</strong>
        <ul>
          <li>It answers the ask head-on: the address is the largest text on the card, and Copy and Open website are labelled buttons rather than icons to discover.</li>
          <li>Account and Service stop looking like settings someone might edit; the footer’s lock line already says nothing here changes.</li>
          <li>The token, the value you rarely need, moves below and keeps today’s Copy / Show grammar exactly.</li>
          <li>Every redesigned frame also adds <b>Open website</b> to the ⋯ menu — the counterpart of a local folder’s Reveal in Finder. The repository rows’ website link (knob) is optional.</li>
        </ul>
      </div>

      {shown.map((direction) => (
        <div key={direction.id} className="spec-direction">
          <h2 className="spec-col-title">{direction.title}</h2>
          <p className="spec-col-note">{direction.note}</p>
          <div className="spec-frame" style={{ width: WIDTHS[knobs.width] }}>
            <Page key={`${direction.id}-${frameKey}`} knobs={knobs} dir={direction.id} />
          </div>
        </div>
      ))}

      <h2 className="spec-h2">How B landed</h2>
      <div className="spec-panel">
        <ol>
          <li><b>Mallard Cloud only, without naming it in the form.</b> The workspace hands <code>ManagedCloudAddress</code> to a Mallard Cloud storage (<code>isManagedCloudStorage</code>, <code>managedCloudAccount</code>, as the connect dialog does), and the shared locked form takes it as a <code>lead</code> band, leaving out the Account and Service fields it states. No DTO change; every other provider’s card is as it was.</li>
          <li><b>Copy</b> uses the shared <code>writeClipboardText</code> with the Cloud panel’s two-second “Copied”; <b>Open</b> uses <code>openExternalUrl</code>, as Setup guide and “Open on website” already do.</li>
          <li><b>⋯ menu:</b> “Open website”, the host as its detail.</li>
          <li><b>Token help</b> drops the internal word “lane”: “Issued at sign-in and valid 90 days. Reconnect to renew it.”</li>
          <li><b>Repository website links</b> were left out: each needs the repository’s <code>website_url</code>, and this page lists bundles, which do not carry it.</li>
        </ol>
      </div>
      <Toast />
    </div>
  );
}

document.documentElement.dataset.theme = readKnobs().theme;
createRoot(document.getElementById("root")!).render(<StrictMode><Specimen /></StrictMode>);
