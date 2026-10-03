import { StrictMode, useEffect, useState, type ReactNode } from "react";
import { createRoot } from "react-dom/client";
import Icon, { storageIconName } from "../src/components/Icons";
import { NoticeChip } from "../src/components/notice/Notice";
import LandingPicker, { type LandingCard } from "../src/components/project-sync/LandingPicker";
import { compactProjectPath, projectLabel, providerLabel } from "../src/components/project-sync/model";
import ProjectPicker from "../src/components/project-sync/ProjectPicker";
import { storageConnectionIssue, storageSubtitle } from "../src/components/project-sync/StorageSettings";
import ProviderMark from "../src/components/sync-review/ProviderMark";
import type { LocalProjectSummary, StorageSummary } from "../src/types";
import "../src/App.css";
import "./project-storages-mockup.css";

/**
 * Mockup: the linked storages on the Projects landing's cards.
 *
 * Today a card's last line is the generic database glyph and the linked
 * storages' names joined by " · ": the glyph says nothing about the transport,
 * a long pair breaks inside a name, and nothing marks where one storage ends.
 *
 *   Before  what shipped until 2026-10-03, rebuilt here: glyph + joined names
 *   A       Tiles — one token per storage: the path bar's tile and the name
 *   B       Shelf — a band at the card's foot, one row per storage with its location
 *   C       Stack — overlapping tiles, the storage the project opens on, +N; SHIPPED
 *
 * C is the real ProjectPicker. Before, A and B are the real LandingPicker with
 * the card's detail swapped. Every fact is registry-level (`StorageSummary`
 * carries provider, summary fields and connection health); "opens on" is the
 * remembered storage ProjectSync keeps per project, falling back to the first
 * link.
 *
 * `npm run dev`, then /mockups/project-storages-mockup.html
 * (`?theme=light|dark&width=600|740|1000&dir=all|before|a|b|c`). Sample data
 * only: no Tauri calls, no provider homes, no storage data.
 */

type Dir = "before" | "a" | "b" | "c";
const DIRS: readonly Dir[] = ["before", "a", "b", "c"];
const DIR_META: Record<Dir, { title: string; tag: string; note: string }> = {
  before: {
    title: "Before",
    tag: "until 2026-10-03",
    note: "The generic database glyph, then every linked name joined by \" · \" on one wrapping line. The glyph says nothing about the transport, a pair breaks inside a name, nothing marks where one storage ends, and the card's accessible name is the project alone — the storages are never announced.",
  },
  a: {
    title: "A · Tiles",
    tag: "not picked",
    note: "One token per linked storage: the provider artwork on the path bar's tile (14px art, 2px pad, ink box) and the name. Tokens wrap whole and a long name ends in an ellipsis. A storage that needs attention says so in a word, never in its tile's colour. The storage the project opens on comes first, in ink.",
  },
  b: {
    title: "B · Shelf",
    tag: "not picked",
    note: "A band at the card's foot, one row per storage: tile, name, and where it lives — folder, bucket, account — in the identifier face. Answers \"which bucket\" without opening the project. Three rows, then \"+N more\". Cards grow with their storages, and a grid row grows with its tallest card.",
  },
  c: {
    title: "C · Stack",
    tag: "shipped",
    note: "One line whatever the count: the provider tiles overlap, then the storage the project opens on, then +N. Every card keeps one height. The other names live only in the tooltip and the accessible name.",
  },
};

/* ——— sample registry ——————————————————————————————————————————— */

function storage(
  storage_id: string,
  name: string,
  provider: string,
  provider_label: string,
  icon: string,
  field: { label: string; value: string; is_path?: boolean },
  extra: Partial<StorageSummary> = {},
): StorageSummary {
  return {
    storage_id,
    name,
    provider,
    config_version: 1,
    provider_label,
    icon,
    available: true,
    summary: [{ field_id: field.label.toLowerCase(), label: field.label, value: field.value, is_path: field.is_path ?? false }],
    settings: {},
    secrets: [],
    connection: null,
    ...extra,
  };
}

const STORAGES: StorageSummary[] = [
  storage("s-local", "myconfstorage", "local", "Local folder", "folder", { label: "Folder", value: "/Users/sample/Documents/myconfstorage", is_path: true }),
  storage("s-feishu", "Feishu storage 3", "feishu", "Feishu Drive", "cloud", { label: "Folder", value: "Mallard sync" }),
  storage("s-feishu-team", "Feishu team space", "feishu", "Feishu Drive", "cloud", { label: "Folder", value: "Team / Mallard" }, {
    connection: { state: "reconnect_required", code: "reconnect_required", message: "Sign in to Feishu again.", observed_at: 0 },
  }),
  storage("s-cloud", "Mallard Cloud rika321", "managed_cloud", "Mallard Cloud", "cloud", { label: "Account", value: "rika321" }),
  storage("s-r2", "R2 mallard-open", "r2", "Cloudflare R2", "cloud", { label: "Bucket", value: "mallard-open" }),
  storage("s-dropbox", "Studio shared Dropbox — game assets archive", "dropbox", "Dropbox", "cloud", { label: "Folder", value: "/Apps/Mallard/game-assets" }),
  storage("s-webdav", "Old WebDAV share", "webdav", "WebDAV", "drive", { label: "Server", value: "dav.example.org" }, { available: false }),
];

function project(
  id: string,
  name: string,
  root: string,
  git: boolean,
  links: string[],
): LocalProjectSummary {
  return {
    local_project_id: id,
    bundle_id: `bundle-${id}`,
    display_name: name,
    revision: 1,
    project_root: root,
    profile_ids: { claude: "profile-claude", codex: "profile-codex" },
    linked_storage_ids: links,
    is_git_repository: git,
  };
}

const PROJECTS: LocalProjectSummary[] = [
  project("p-horror", "HorrorGame", "/Users/sample/Desktop/project/HorrorGame", true, ["s-local", "s-feishu", "s-cloud"]),
  project("p-demo", "mallard_test_demo", "/Users/sample/Desktop/project/demo/mallard_test_demo", true, []),
  project("p-echo", "echo-park-horror", "/Users/sample/Desktop/project/demo/HorrorGame", true, ["s-feishu-team", "s-local"]),
  project("p-app", "mallard-app", "/Users/sample/Desktop/project/mallard-app", true, ["s-cloud"]),
  project("p-pelican", "mallard-with-pelican", "/Users/sample/Desktop/project/mallard-with-pelican", true, ["s-r2", "s-cloud"]),
  project("p-pelican-2", "mallard-with-pelican (2)", "/Users/sample/Desktop/project/demo/mallard-with-pelican", true, ["s-r2"]),
  project("p-nongit", "mallardNonGit", "/Users/sample/Desktop/project/mallardNonGit", false, ["s-dropbox"]),
  project("p-internal", "mallardDevInternal", "/Users/sample/Desktop/project/mallardDevInternal", false, ["s-cloud", "s-webdav", "s-r2", "s-local"]),
];

/** The remembered active storage per project — ProjectSync's
 *  `activeStorageByProject`. Absent, a project opens on its first link. */
const REMEMBERED: Record<string, string> = { "p-horror": "s-feishu" };

const storageById = new Map(STORAGES.map((candidate) => [candidate.storage_id, candidate]));
const noop = () => {};

/** Linked storages, the one the project opens on first. */
function linkedStorages(target: LocalProjectSummary): { linked: StorageSummary[]; opensOn: string | null } {
  const linked = (target.linked_storage_ids ?? [])
    .map((storageId) => storageById.get(storageId))
    .filter((candidate): candidate is StorageSummary => Boolean(candidate));
  const remembered = REMEMBERED[target.local_project_id];
  const opensOn = linked.some((candidate) => candidate.storage_id === remembered)
    ? remembered
    : linked[0]?.storage_id ?? null;
  return {
    linked: [...linked].sort((left, right) => Number(right.storage_id === opensOn) - Number(left.storage_id === opensOn)),
    opensOn,
  };
}

/** The one word a storage needing attention carries: colour stays the transport. */
function attentionWord(target: StorageSummary): string | null {
  if (!target.available) return "Unavailable";
  if (storageConnectionIssue(target)) return "Reconnect";
  return null;
}

function storageMark(target: StorageSummary) {
  return <Icon name={storageIconName(target.provider, target.icon)} size={14} />;
}

/* ——— the three details ——————————————————————————————————————————— */

function NoStorage({ className }: { className: string }) {
  return (
    <span className={`${className} none`}>
      <Icon name="link-off" size={12} />
      <span>No storage linked</span>
    </span>
  );
}

function TilesDetail({ linked, opensOn }: { linked: StorageSummary[]; opensOn: string | null }) {
  if (linked.length === 0) return <span className="pst-a"><NoStorage className="pst-a-item" /></span>;
  return (
    <span className="pst-a">
      {linked.map((target) => {
        const word = attentionWord(target);
        return (
          <span
            key={target.storage_id}
            className={`pst-a-item${target.storage_id === opensOn ? " opens" : ""}${word ? " attention" : ""}`}
            title={`${target.name} · ${storageSubtitle(target)}`}
          >
            {storageMark(target)}
            <span className="pst-a-name">{target.name}</span>
            {word && <span className="pst-a-word"><Icon name="alert-triangle" size={12} />{word}</span>}
          </span>
        );
      })}
    </span>
  );
}

const SHELF_ROWS = 3;

function ShelfDetail({ linked, opensOn }: { linked: StorageSummary[]; opensOn: string | null }) {
  if (linked.length === 0) return <span className="pst-b"><NoStorage className="pst-b-row" /></span>;
  const shown = linked.length > SHELF_ROWS ? linked.slice(0, SHELF_ROWS - 1) : linked;
  const rest = linked.length - shown.length;
  return (
    <span className="pst-b">
      {shown.map((target) => {
        const word = attentionWord(target);
        return (
          <span key={target.storage_id} className={`pst-b-row${target.storage_id === opensOn ? " opens" : ""}`}>
            {storageMark(target)}
            <span className="pst-b-name">{target.name}</span>
            {word
              ? <NoticeChip severity="warning">{word}</NoticeChip>
              : <span className="pst-b-where">{storageSubtitle(target)}</span>}
          </span>
        );
      })}
      {rest > 0 && <span className="pst-b-more">+{rest} more</span>}
    </span>
  );
}

/* ——— cards: ProjectPicker's adapter, the detail swapped ———————————— */

function cardsFor(dir: "before" | "a" | "b"): LandingCard[] {
  return PROJECTS.map((target) => {
    const label = projectLabel(target);
    const { linked, opensOn } = linkedStorages(target);
    if (dir === "before") {
      const names = (target.linked_storage_ids ?? [])
        .map((storageId) => storageById.get(storageId)?.name)
        .filter((name): name is string => Boolean(name));
      return {
        key: target.local_project_id,
        icon: target.is_git_repository === true ? "git-folder" : "folder",
        title: label,
        ariaLabel: target.is_git_repository === true ? `${label}, Git repository` : label,
        marks: Object.entries(target.profile_ids ?? {}).map(([provider]) => (
          <ProviderMark key={provider} provider={provider} displayName={providerLabel(provider)} />
        )),
        location: compactProjectPath(target.project_root),
        detail: (
          <>
            <Icon name="storage" size={13} />
            {names.length === 0 ? "No storage linked" : names.join(" · ")}
          </>
        ),
        detailMuted: names.length === 0,
        onOpen: noop,
      };
    }
    const detail: ReactNode = dir === "a"
      ? <TilesDetail linked={linked} opensOn={opensOn} />
      : <ShelfDetail linked={linked} opensOn={opensOn} />;
    /* The storages join the door's name: today's aria-label is the project
       alone, so a screen reader never hears where it syncs. */
    const storagesSpoken = linked.length === 0
      ? "No storage linked"
      : `Storages: ${linked.map((target) => {
        const word = attentionWord(target);
        return [target.name, target.storage_id === opensOn && linked.length > 1 ? "opens here" : null, word]
          .filter(Boolean).join(", ");
      }).join("; ")}`;
    return {
      key: target.local_project_id,
      icon: target.is_git_repository === true ? "git-folder" : "folder",
      title: label,
      ariaLabel: `${label}${target.is_git_repository === true ? ", Git repository" : ""}. ${storagesSpoken}`,
      marks: Object.entries(target.profile_ids ?? {}).map(([provider]) => (
        <ProviderMark key={provider} provider={provider} displayName={providerLabel(provider)} />
      )),
      location: compactProjectPath(target.project_root),
      detail,
      onOpen: noop,
    };
  });
}

/* ——— page ——————————————————————————————————————————————————————— */

function Segmented<T extends string>({ label, value, options, onChange }: {
  label: string;
  value: T;
  options: [T, string][];
  onChange: (value: T) => void;
}) {
  return (
    <span className="pst-group">
      <span className="pst-label">{label}</span>
      <span className="sync-segmented" role="group" aria-label={label}>
        {options.map(([key, text]) => (
          <button key={key} type="button" className={value === key ? "active" : undefined} aria-pressed={value === key} onClick={() => onChange(key)}>
            {text}
          </button>
        ))}
      </span>
    </span>
  );
}

function Frame({ dir, width }: { dir: Dir; width: number }) {
  const meta = DIR_META[dir];
  return (
    <section className="pst-frame" data-dir={dir}>
      <header className="pst-frame-head">
        <h2>{meta.title}</h2>
        <span className={`pst-tag${meta.tag === "shipped" ? " pick" : ""}`}>{meta.tag}</span>
        <p>{meta.note}</p>
      </header>
      <div className="pst-window" style={{ width: `${width + 64}px` }}>
        <main className="sync-main sync-project-links-page sync-landing-page">
          {dir === "c" ? (
            <ProjectPicker
              projects={PROJECTS}
              profiles={[]}
              storages={STORAGES}
              rememberedStorageByProject={REMEMBERED}
              onAddProject={noop}
              onAddCloudProject={noop}
              onSelectProject={noop}
            />
          ) : (
            <LandingPicker
              title="Projects"
              count={`${PROJECTS.length} on this machine`}
              actionLabel="Add project…"
              onAction={noop}
              cards={cardsFor(dir)}
            />
          )}
        </main>
      </div>
    </section>
  );
}

function Preview() {
  const params = new URLSearchParams(window.location.search);
  const [theme, setTheme] = useState<"light" | "dark">(params.get("theme") === "dark" ? "dark" : "light");
  const [width, setWidth] = useState<"600" | "740" | "1000">(
    params.get("width") === "600" ? "600" : params.get("width") === "1000" ? "1000" : "740",
  );
  const initialDir = params.get("dir");
  const [show, setShow] = useState<"all" | Dir>(
    initialDir && (DIRS as readonly string[]).includes(initialDir) ? initialDir as Dir : "all",
  );

  useEffect(() => {
    document.documentElement.dataset.theme = theme;
  }, [theme]);

  useEffect(() => {
    const next = new URLSearchParams({ theme, width, dir: show });
    window.history.replaceState(null, "", `?${next}`);
  }, [theme, width, show]);

  return (
    <>
      <div className="pst-controls">
        <strong>Project storages</strong>
        <Segmented label="Show" value={show} onChange={setShow} options={[["all", "All"], ["before", "Before"], ["a", "A"], ["b", "B"], ["c", "C"]]} />
        <Segmented label="Page" value={width} onChange={setWidth} options={[["600", "600 · 2×294"], ["740", "740 · 2×364"], ["1000", "1000 · 3×325"]]} />
        <Segmented label="Theme" value={theme} onChange={setTheme} options={[["light", "Light"], ["dark", "Dark"]]} />
        <span className="pst-status">Sample data · no Tauri calls</span>
      </div>
      <div className="pst-stage">
        {DIRS.filter((dir) => show === "all" || show === dir).map((dir) => (
          <Frame key={dir} dir={dir} width={Number(width)} />
        ))}
        <section className="pst-frame pst-questions">
          <header className="pst-frame-head">
            <h2>Open questions</h2>
          </header>
          <ul>
            <li>Names repeat the transport: "Mallard Cloud rika321" beside the duck tile, "R2 mallard-open" beside the R2 tile. Mallard Cloud's default name is generated as <code>Mallard Cloud &#123;login&#125;</code>. Every direction shows the stored name unchanged.</li>
            <li>"Opens on" is ProjectSync's remembered <code>activeStorageByProject</code>, passed to <code>ProjectPicker</code> as <code>rememberedStorageByProject</code>; a project with none opens on its first link.</li>
            <li>Out of scope: the Claude and Codex marks are the same on every card here.</li>
          </ul>
        </section>
      </div>
    </>
  );
}

const previewRoot = createRoot(document.getElementById("root")!);
previewRoot.render(<StrictMode><Preview /></StrictMode>);
if (import.meta.hot) import.meta.hot.dispose(() => previewRoot.unmount());
