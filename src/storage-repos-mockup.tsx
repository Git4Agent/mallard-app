import { StrictMode, useState, type ReactNode } from "react";
import { createRoot, type Root } from "react-dom/client";
import Icon from "./components/Icons";
import {
  CatalogStorageRepositoryRow,
  StorageRepositoryRow,
} from "./components/project-sync/ProjectLinksWorkspace";
import type { CatalogStorageRepository } from "./components/storage-catalog/types";
import type { RemoteBundleSummary } from "./types";
import "./App.css";
import "./RetroTheme.css";

/**
 * Specimen: the storage page's Repositories card listing the Catalog's kind
 * repositories beside the project ones. Both row flavours are the shipped
 * components over the shipped stylesheets; only the sample data is local.
 * The design: the build derives every repository id it can read —
 * for_kind(provider, kind) over the ownership matrix — and inspects each:
 * absent means no row, present means the project row's grammar, and
 * present-but-unreadable means a visible refusal row, never a silent hole.
 * Not part of the app bundle — dev server only.
 */

const NOW = Math.floor(Date.now() / 1000);

const PROJECTS: RemoteBundleSummary[] = [
  {
    bundle_id: "repo-9f2c41d6a8b35e07",
    display_name: "HorrorGame",
    revision: 1,
    updated_at: NOW - 2 * 3600,
    resource_count: 12,
    providers: ["codex"],
    repository_fingerprint: null,
  },
  {
    bundle_id: "repo-c81d570b2f9e4a63",
    display_name: "mallard-app",
    revision: 1,
    updated_at: NOW - 2 * 3600 - 240,
    resource_count: 31,
    providers: ["codex"],
    repository_fingerprint: "github.com/Git4Agent/mallard-app",
  },
];

const SKILLS_CODEX: CatalogStorageRepository = {
  repository_id: "library-3a91f04c77d2b8e6a5c31f9d04b7e28c",
  provider: "codex",
  resource_kind: "agent_skill",
  revision: 10,
  updated_at: NOW - 2 * 3600 + 300,
  resource_names: ["changelog-writer", "deploy-checklist", "triage"],
  error: null,
};

const PETS_V2: CatalogStorageRepository = {
  repository_id: "library-b57e20d94c6f13a8e072c5b9d1f4a36e",
  provider: "codex",
  resource_kind: "codex_pet_v2",
  revision: 5,
  updated_at: NOW - 3 * 3600,
  resource_names: ["Dahuang", "Maple", "Patch"],
  error: null,
};

const PLUGINS_CLAUDE: CatalogStorageRepository = {
  repository_id: "library-6c04d8f1a93b57e2c46f0a812d95e3b7",
  provider: "claude",
  resource_kind: "plugin_install_intent",
  revision: 2,
  updated_at: NOW - 26 * 3600,
  resource_names: ["context7", "sqlfluff"],
  error: null,
};

const UNREADABLE: CatalogStorageRepository = {
  repository_id: "library-e29a51c8d40b76f3a1c58e02b9d47f61",
  provider: "codex",
  resource_kind: "codex_pet_v1",
  revision: null,
  updated_at: null,
  resource_names: [],
  error:
    "library tag 'library-e29a51c8d40b76f3a1c58e02b9d47f61' does not match its provider and kind derivation",
};

function RepositoriesCard({ projects, catalog }: {
  projects: RemoteBundleSummary[];
  catalog: CatalogStorageRepository[];
}) {
  return (
    <section className="sync-storage-repositories" aria-label="Repositories">
      <div className="sync-storage-repositories-heading">
        <h2>Repositories</h2>
      </div>
      {projects.length === 0 && catalog.length === 0 ? (
        <div className="sync-storage-repository-state">No repositories in this storage.</div>
      ) : (
        <div className="sync-storage-repository-list">
          {projects.map((bundle) => (
            <StorageRepositoryRow key={bundle.bundle_id} bundle={bundle} />
          ))}
          {catalog.map((repo) => (
            <CatalogStorageRepositoryRow key={repo.repository_id} repo={repo} />
          ))}
        </div>
      )}
    </section>
  );
}

function Panel({ title, note, children }: { title: string; note: ReactNode; children: ReactNode }) {
  return (
    <section className="mockup-panel">
      <h3>{title}</h3>
      <p>{note}</p>
      <div className="mockup-stage">{children}</div>
    </section>
  );
}

function Harness() {
  const [dark, setDark] = useState(
    () => document.documentElement.getAttribute("data-theme") === "dark",
  );
  return (
    <div className="app mockup-root">
      <div className="mockup-shell">
        <header className="mockup-head">
          <div>
            <h1>Catalog repositories in the storage card</h1>
            <p>
              The Repositories card lists everything the build can read out of the storage: the
              project repositories it lists today, and the Catalog’s kind repositories — one per
              (provider, kind), derived, never scanned. Catalog rows wear the project row’s exact
              grammar; their identity is the kind icon and the scope-label name, and their fold
              names what the set holds. Open a chevron to see the fold.
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
          title="One card, every repository"
          note={<>
            Projects first — they are the rows a person comes looking for — then the Catalog’s
            sets, each internally newest-first. Same chips, same fold affordance; the fold holds
            the ID, the absolute stamp, the count, and the contents by name — the fact the
            Catalog page can’t show you about a storage you haven’t linked.
          </>}
        >
          <RepositoriesCard projects={PROJECTS} catalog={[SKILLS_CODEX, PETS_V2, PLUGINS_CLAUDE]} />
        </Panel>

        <Panel
          title="A repository the build can’t read"
          note={<>
            A derived id that answers with a broken tag or manifest fails closed on the Catalog
            page — here it fails <em>visible</em>: the row states the refusal where the chips
            would be, and the fold carries the backend’s sentence. A silent hole would read as
            “nothing stored”, which is exactly wrong.
          </>}
        >
          <RepositoriesCard projects={[]} catalog={[PETS_V2, UNREADABLE]} />
        </Panel>

        <Panel
          title="Catalog only"
          note={<>
            A storage that holds Catalog sets but no project yet stops claiming “No repositories
            in this storage” — the empty line is reserved for a storage that truly holds nothing
            this build can read.
          </>}
        >
          <RepositoriesCard projects={[]} catalog={[SKILLS_CODEX, PLUGINS_CLAUDE]} />
        </Panel>
      </div>
    </div>
  );
}

const SHELL_CSS = `
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
.mockup-stage { padding: 18px; border: 1px solid var(--border); border-radius: 6px; background: var(--app-shell); }
body { background: var(--app-shell); }
`;

/* ?theme=dark so a headless capture can take the dark panels without clicking. */
const START_DARK = new URLSearchParams(location.search).get("theme") === "dark";
document.documentElement.setAttribute("data-theme", START_DARK ? "dark" : "light");
const style = document.createElement("style");
style.textContent = SHELL_CSS;
document.head.appendChild(style);
const container = document.getElementById("root")! as HTMLElement & { _root?: Root };
container._root ??= createRoot(container);
container._root.render(<StrictMode><Harness /></StrictMode>);
