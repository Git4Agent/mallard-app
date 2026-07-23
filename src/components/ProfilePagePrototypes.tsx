import { useEffect, useState, type CSSProperties, type ReactNode } from "react";
import type { AppTheme } from "../types";
import Icon from "./Icons";
import "./ProfilePagePrototypes.css";

type ConceptId = 1 | 2 | 3 | 4 | 5;
type ProfileTab = "history" | "skills" | "plugins";

interface Props {
  theme: AppTheme;
  onThemeChange: (theme: AppTheme) => void;
}

interface Concept {
  id: ConceptId;
  name: string;
  note: string;
  accent: string;
}

const CONCEPTS: Concept[] = [
  {
    id: 1,
    name: "Status rail",
    note: "A calm evolution of the current page with stronger hierarchy and sync context.",
    accent: "#78a9ff",
  },
  {
    id: 2,
    name: "Split inspector",
    note: "History stays primary while project configuration lives in a persistent inspector.",
    accent: "#a78bfa",
  },
  {
    id: 3,
    name: "Command strip",
    note: "A compact, keyboard-minded workspace for frequent pull and push workflows.",
    accent: "#f0a960",
  },
  {
    id: 4,
    name: "Project passport",
    note: "A more expressive identity surface with scannable project facts.",
    accent: "#55c7a4",
  },
  {
    id: 5,
    name: "Activity first",
    note: "Git and session activity lead; sync controls become a focused action dock.",
    accent: "#f08a8a",
  },
];

const tabs: Array<{ id: ProfileTab; label: string; icon: "git-branch" | "folder" | "link" }> = [
  { id: "history", label: "Git & sessions", icon: "git-branch" },
  { id: "skills", label: "Skills", icon: "folder" },
  { id: "plugins", label: "Plugins", icon: "link" },
];

function initialConcept(): ConceptId {
  const value = Number.parseInt(
    new URLSearchParams(window.location.search).get("profile-prototype") ?? "1",
    10,
  );
  return value >= 1 && value <= 5 ? value as ConceptId : 1;
}

function updateConceptUrl(concept: ConceptId) {
  const url = new URL(window.location.href);
  url.searchParams.set("profile-prototype", String(concept));
  window.history.replaceState(null, "", url);
}

function ProjectFacts({ condensed = false }: { condensed?: boolean }) {
  return (
    <div className={`pp-project-facts${condensed ? " condensed" : ""}`}>
      <span title="Project folder">
        <Icon name="folder" size={13} />
        ~/Desktop/project/demo/mallard_test_demo
      </span>
      <span title="Agent profile">
        <Icon name="terminal" size={13} />
        Default Codex
      </span>
      <span title="Current branch">
        <Icon name="git-branch" size={13} />
        main
      </span>
    </div>
  );
}

function ProjectTitle({ eyebrow }: { eyebrow?: ReactNode }) {
  return (
    <div className="pp-project-title">
      {eyebrow}
      <div className="pp-project-name-row">
        <h1>mallard_test_demo</h1>
        <button type="button" className="pp-text-button">Rename</button>
      </div>
      <ProjectFacts />
    </div>
  );
}

function ProfileTabs({
  activeTab,
  onChange,
  vertical = false,
}: {
  activeTab: ProfileTab;
  onChange: (tab: ProfileTab) => void;
  vertical?: boolean;
}) {
  return (
    <div
      className={`pp-tabs${vertical ? " vertical" : ""}`}
      role="tablist"
      aria-label="Project profile sections"
      aria-orientation={vertical ? "vertical" : "horizontal"}
    >
      {tabs.map((tab) => (
        <button
          key={tab.id}
          type="button"
          role="tab"
          aria-selected={activeTab === tab.id}
          className={activeTab === tab.id ? "active" : undefined}
          onClick={() => onChange(tab.id)}
        >
          <Icon name={tab.icon} size={15} />
          <span>{tab.label}</span>
          {tab.id !== "history" && <small>{tab.id === "skills" ? 4 : 2}</small>}
        </button>
      ))}
    </div>
  );
}

function SyncActions({
  onAction,
  compact = false,
}: {
  onAction: (action: "pull" | "push") => void;
  compact?: boolean;
}) {
  return (
    <div className={`pp-sync-actions${compact ? " compact" : ""}`} role="group" aria-label="Sync actions">
      <button type="button" className="pp-sync-secondary" onClick={() => onAction("pull")}>
        <Icon name="download" size={15} />
        Pull
      </button>
      <button type="button" className="pp-sync-primary" onClick={() => onAction("push")}>
        <Icon name="upload" size={15} />
        Push
      </button>
    </div>
  );
}

function StorageIdentity({ detailed = false }: { detailed?: boolean }) {
  return (
    <div className={`pp-storage-identity${detailed ? " detailed" : ""}`}>
      <span className="pp-storage-icon"><Icon name="drive" size={21} /></span>
      <span className="pp-storage-copy">
        <strong>Local storage 2</strong>
        <span>~/Desktop/project/myconfstorage2</span>
      </span>
      <span className="pp-linked-count">2 linked</span>
    </div>
  );
}

function ContentPanel({ activeTab, roomy = false }: { activeTab: ProfileTab; roomy?: boolean }) {
  if (activeTab === "skills") {
    return (
      <section className={`pp-content-panel${roomy ? " roomy" : ""}`} aria-labelledby="pp-skills-heading">
        <div className="pp-section-heading">
          <div>
            <span className="pp-kicker">PROJECT RESOURCES</span>
            <h2 id="pp-skills-heading">Skills</h2>
          </div>
          <span className="pp-heading-meta">4 available · 3 selected</span>
        </div>
        <div className="pp-resource-list">
          {["frontend-skill", "playwright", "pdf", "openai-docs"].map((skill, index) => (
            <div className="pp-resource-row" key={skill}>
              <span className="pp-resource-icon"><Icon name="tool" size={15} /></span>
              <strong>{skill}</strong>
              <span>{index === 2 ? "Available" : "Included in next push"}</span>
              <Icon name={index === 2 ? "plus" : "check-circle"} size={15} />
            </div>
          ))}
        </div>
      </section>
    );
  }

  if (activeTab === "plugins") {
    return (
      <section className={`pp-content-panel${roomy ? " roomy" : ""}`} aria-labelledby="pp-plugins-heading">
        <div className="pp-section-heading">
          <div>
            <span className="pp-kicker">PROJECT RESOURCES</span>
            <h2 id="pp-plugins-heading">Plugins</h2>
          </div>
          <span className="pp-heading-meta">2 configured</span>
        </div>
        <div className="pp-resource-list">
          {[
            ["GitHub", "Connected for this project"],
            ["Understand Anything", "Local knowledge graph ready"],
          ].map(([name, detail]) => (
            <div className="pp-resource-row" key={name}>
              <span className="pp-resource-icon"><Icon name="link" size={15} /></span>
              <strong>{name}</strong>
              <span>{detail}</span>
              <Icon name="check-circle" size={15} />
            </div>
          ))}
        </div>
      </section>
    );
  }

  return (
    <section className={`pp-content-panel${roomy ? " roomy" : ""}`} aria-labelledby="pp-history-heading">
      <div className="pp-section-heading">
        <div>
          <span className="pp-kicker">WORKING TREE</span>
          <h2 id="pp-history-heading">Git history</h2>
        </div>
        <div className="pp-branch-status">
          <Icon name="git-branch" size={13} />
          <strong>main</strong>
          <span>Current</span>
          <button type="button" aria-label="Refresh history"><Icon name="refresh" size={14} /></button>
        </div>
      </div>

      <div className="pp-history-group">
        <div className="pp-history-group-label">
          <span>Uncommitted</span>
          <small>2 files changed</small>
        </div>
        <button type="button" className="pp-history-row">
          <span className="pp-history-mark active"><Icon name="activity" size={14} /></span>
          <span className="pp-history-copy">
            <strong>OpenAI hackathon</strong>
            <small>Working tree · moments ago</small>
          </span>
          <span className="pp-history-state">Local only</span>
          <Icon name="chevron-right" size={14} />
        </button>
      </div>

      <div className="pp-history-group pp-history-recent">
        <div className="pp-history-group-label">
          <span>Recent sessions</span>
          <small>Today</small>
        </div>
        <button type="button" className="pp-history-row">
          <span className="pp-history-mark"><Icon name="message" size={14} /></span>
          <span className="pp-history-copy">
            <strong>Refine project profile layout</strong>
            <small>Codex · 36 minutes ago</small>
          </span>
          <span className="pp-history-state synced">Synced</span>
          <Icon name="chevron-right" size={14} />
        </button>
      </div>
    </section>
  );
}

function AppBar({
  theme,
  onThemeChange,
  activity,
}: {
  theme: AppTheme;
  onThemeChange: (theme: AppTheme) => void;
  activity: string;
}) {
  return (
    <header className="pp-appbar">
      <div className="pp-appbar-brand">
        <span className="pp-brand-mark"><Icon name="git-folder" size={16} /></span>
        <strong>mallard_test_demo</strong>
      </div>
      <div className="pp-appbar-actions">
        <span className="pp-activity"><i />{activity}</span>
        <button
          type="button"
          className="pp-theme-button"
          onClick={() => onThemeChange(theme === "dark" ? "light" : "dark")}
        >
          {theme === "dark" ? "Light theme" : "Dark theme"}
        </button>
      </div>
    </header>
  );
}

function ConceptOne({
  activeTab,
  onTabChange,
  onAction,
}: {
  activeTab: ProfileTab;
  onTabChange: (tab: ProfileTab) => void;
  onAction: (action: "pull" | "push") => void;
}) {
  return (
    <main className="pp-canvas pp-status-rail">
      <section className="pp-status-hero">
        <ProjectTitle
          eyebrow={(
            <div className="pp-status-eyebrow">
              <span><i /> Ready to sync</span>
              <span>Last synced 4 min ago</span>
            </div>
          )}
        />
        <div className="pp-hero-branch">
          <span>BRANCH</span>
          <strong><Icon name="git-branch" size={14} /> main</strong>
          <small>up to date</small>
        </div>
      </section>

      <section className="pp-status-storage" aria-label="Active storage">
        <StorageIdentity detailed />
        <button type="button" className="pp-icon-button" aria-label="Storage actions">
          <Icon name="more" size={17} />
        </button>
        <SyncActions onAction={onAction} />
      </section>

      <ProfileTabs activeTab={activeTab} onChange={onTabChange} />
      <ContentPanel activeTab={activeTab} />
    </main>
  );
}

function ConceptTwo({
  activeTab,
  onTabChange,
  onAction,
}: {
  activeTab: ProfileTab;
  onTabChange: (tab: ProfileTab) => void;
  onAction: (action: "pull" | "push") => void;
}) {
  return (
    <main className="pp-canvas pp-split-inspector">
      <section className="pp-split-main">
        <div className="pp-split-heading">
          <ProjectTitle
            eyebrow={(
              <span className="pp-overline"><i /> ACTIVE PROJECT</span>
            )}
          />
          <span className="pp-split-sync-note"><Icon name="check-circle" size={14} /> All checks passed</span>
        </div>
        <ProfileTabs activeTab={activeTab} onChange={onTabChange} />
        <ContentPanel activeTab={activeTab} roomy />
      </section>

      <aside className="pp-inspector" aria-label="Project connection inspector">
        <div className="pp-inspector-heading">
          <span>CONNECTION</span>
          <button type="button" className="pp-icon-button" aria-label="Connection settings">
            <Icon name="settings" size={15} />
          </button>
        </div>
        <div className="pp-inspector-storage">
          <StorageIdentity detailed />
          <button type="button" className="pp-storage-switch">
            Switch storage <Icon name="chevron-down" size={13} />
          </button>
        </div>
        <dl className="pp-inspector-facts">
          <div><dt>Agent</dt><dd><Icon name="terminal" size={13} /> Default Codex</dd></div>
          <div><dt>Branch</dt><dd><Icon name="git-branch" size={13} /> main</dd></div>
          <div><dt>Bundle</dt><dd><i /> Ready</dd></div>
          <div><dt>Linked</dt><dd>2 projects</dd></div>
        </dl>
        <div className="pp-inspector-callout">
          <Icon name="badge-check" size={17} />
          <span><strong>Safe to sync</strong>No path or credential warnings</span>
        </div>
        <SyncActions onAction={onAction} />
      </aside>
    </main>
  );
}

function ConceptThree({
  activeTab,
  onTabChange,
  onAction,
}: {
  activeTab: ProfileTab;
  onTabChange: (tab: ProfileTab) => void;
  onAction: (action: "pull" | "push") => void;
}) {
  return (
    <main className="pp-canvas pp-command-strip">
      <div className="pp-command-context">
        <div className="pp-command-breadcrumb">
          <span>Projects</span><Icon name="chevron-right" size={12} />
          <strong>mallard_test_demo</strong>
          <button type="button" aria-label="Rename project"><Icon name="settings" size={13} /></button>
        </div>
        <ProjectFacts condensed />
      </div>

      <section className="pp-command-bar" aria-label="Project sync command bar">
        <div className="pp-command-storage">
          <span className="pp-command-label">SYNC TARGET</span>
          <StorageIdentity />
          <button type="button" aria-label="Choose storage"><Icon name="chevron-down" size={14} /></button>
        </div>
        <div className="pp-command-readiness">
          <span className="pp-command-label">STATUS</span>
          <strong><i /> Ready</strong>
          <small>2 local changes</small>
        </div>
        <SyncActions onAction={onAction} />
      </section>

      <div className="pp-command-workspace">
        <ProfileTabs activeTab={activeTab} onChange={onTabChange} vertical />
        <ContentPanel activeTab={activeTab} roomy />
      </div>

      <footer className="pp-command-footer">
        <span><Icon name="git-branch" size={12} /> main*</span>
        <span><Icon name="terminal" size={12} /> Default Codex</span>
        <span className="pp-command-shortcut">⌘ ⇧ P <small>Open sync review</small></span>
      </footer>
    </main>
  );
}

function ConceptFour({
  activeTab,
  onTabChange,
  onAction,
}: {
  activeTab: ProfileTab;
  onTabChange: (tab: ProfileTab) => void;
  onAction: (action: "pull" | "push") => void;
}) {
  return (
    <main className="pp-canvas pp-passport">
      <section className="pp-passport-identity">
        <div className="pp-passport-monogram">M</div>
        <div className="pp-passport-title">
          <span>PROJECT PROFILE / 01</span>
          <h1>mallard_<br />test_demo</h1>
          <button type="button" className="pp-text-button">Edit identity</button>
        </div>
        <div className="pp-passport-facts">
          <div>
            <span>01 / LOCATION</span>
            <strong>~/Desktop/project/demo/<br />mallard_test_demo</strong>
          </div>
          <div>
            <span>02 / AGENT</span>
            <strong>Default Codex</strong>
          </div>
          <div>
            <span>03 / BRANCH</span>
            <strong>main <i>Current</i></strong>
          </div>
        </div>
        <div className="pp-passport-sync">
          <span className="pp-passport-ready"><i /> SYNC READY</span>
          <StorageIdentity />
          <SyncActions onAction={onAction} />
        </div>
      </section>

      <div className="pp-passport-content">
        <ProfileTabs activeTab={activeTab} onChange={onTabChange} />
        <ContentPanel activeTab={activeTab} roomy />
      </div>
    </main>
  );
}

function ConceptFive({
  activeTab,
  onTabChange,
  onAction,
}: {
  activeTab: ProfileTab;
  onTabChange: (tab: ProfileTab) => void;
  onAction: (action: "pull" | "push") => void;
}) {
  return (
    <main className="pp-canvas pp-activity-first">
      <aside className="pp-activity-nav">
        <div className="pp-activity-project">
          <span className="pp-activity-project-mark">M</span>
          <div><strong>mallard_test_demo</strong><small>Active project</small></div>
        </div>
        <ProfileTabs activeTab={activeTab} onChange={onTabChange} vertical />
        <div className="pp-activity-profile">
          <span><Icon name="terminal" size={14} /></span>
          <div><small>AGENT PROFILE</small><strong>Default Codex</strong></div>
        </div>
      </aside>

      <section className="pp-activity-workspace">
        <div className="pp-activity-heading">
          <div>
            <span className="pp-overline">PROJECT ACTIVITY</span>
            <h1>{activeTab === "history" ? "What changed" : activeTab === "skills" ? "Project skills" : "Project plugins"}</h1>
            <p>{activeTab === "history" ? "Review local work and synced Codex sessions in one timeline." : "Resources attached to mallard_test_demo."}</p>
          </div>
          <div className="pp-activity-branch">
            <Icon name="git-branch" size={14} />
            <span><small>CURRENT BRANCH</small><strong>main</strong></span>
          </div>
        </div>
        <ContentPanel activeTab={activeTab} roomy />
      </section>

      <aside className="pp-action-dock" aria-label="Sync dock">
        <div className="pp-action-dock-heading">
          <span>SYNC</span>
          <strong><i /> Ready</strong>
        </div>
        <StorageIdentity detailed />
        <div className="pp-action-dock-summary">
          <span>Local changes<strong>2</strong></span>
          <span>Last sync<strong>4m</strong></span>
        </div>
        <SyncActions onAction={onAction} />
        <button type="button" className="pp-dock-settings">
          <Icon name="settings" size={14} /> Storage settings
        </button>
      </aside>
    </main>
  );
}

export default function ProfilePagePrototypes({ theme, onThemeChange }: Props) {
  const [concept, setConcept] = useState<ConceptId>(initialConcept);
  const [activeTab, setActiveTab] = useState<ProfileTab>("history");
  const [activity, setActivity] = useState("Ready to sync");
  const captureMode = new URLSearchParams(window.location.search).get("capture") === "1";
  const selectedConcept = CONCEPTS.find((candidate) => candidate.id === concept) ?? CONCEPTS[0];

  useEffect(() => {
    const handleKeyDown = (event: KeyboardEvent) => {
      if (event.metaKey || event.ctrlKey || event.altKey || event.target instanceof HTMLInputElement) return;
      const numericConcept = Number.parseInt(event.key, 10);
      if (numericConcept < 1 || numericConcept > 5) return;
      const nextConcept = numericConcept as ConceptId;
      setConcept(nextConcept);
      setActiveTab("history");
      updateConceptUrl(nextConcept);
    };
    window.addEventListener("keydown", handleKeyDown);
    return () => window.removeEventListener("keydown", handleKeyDown);
  }, []);

  const chooseConcept = (nextConcept: ConceptId) => {
    setConcept(nextConcept);
    setActiveTab("history");
    setActivity("Ready to sync");
    updateConceptUrl(nextConcept);
  };

  const handleSyncAction = (action: "pull" | "push") => {
    setActivity(action === "pull" ? "Pull review ready" : "Push review ready");
  };

  const conceptStyle = {
    "--pp-accent": selectedConcept.accent,
  } as CSSProperties;

  return (
    <div
      className={`pp-root pp-concept-${concept}${captureMode ? " pp-capture-mode" : ""}`}
      style={conceptStyle}
    >
      <AppBar theme={theme} onThemeChange={onThemeChange} activity={activity} />
      <div className="pp-concept-stage" key={concept}>
        {concept === 1 && (
          <ConceptOne activeTab={activeTab} onTabChange={setActiveTab} onAction={handleSyncAction} />
        )}
        {concept === 2 && (
          <ConceptTwo activeTab={activeTab} onTabChange={setActiveTab} onAction={handleSyncAction} />
        )}
        {concept === 3 && (
          <ConceptThree activeTab={activeTab} onTabChange={setActiveTab} onAction={handleSyncAction} />
        )}
        {concept === 4 && (
          <ConceptFour activeTab={activeTab} onTabChange={setActiveTab} onAction={handleSyncAction} />
        )}
        {concept === 5 && (
          <ConceptFive activeTab={activeTab} onTabChange={setActiveTab} onAction={handleSyncAction} />
        )}
      </div>

      {!captureMode && (
        <nav className="pp-concept-switcher" aria-label="Profile page prototypes">
          <div className="pp-switcher-copy">
            <span>PROFILE PAGE DIRECTIONS</span>
            <strong>{String(concept).padStart(2, "0")} · {selectedConcept.name}</strong>
            <small>{selectedConcept.note}</small>
          </div>
          <div className="pp-switcher-options">
            {CONCEPTS.map((candidate) => (
              <button
                key={candidate.id}
                type="button"
                className={candidate.id === concept ? "active" : undefined}
                aria-current={candidate.id === concept ? "page" : undefined}
                aria-label={`Show prototype ${candidate.id}: ${candidate.name}`}
                onClick={() => chooseConcept(candidate.id)}
              >
                {String(candidate.id).padStart(2, "0")}
              </button>
            ))}
          </div>
          <span className="pp-switcher-hint">Press 1–5</span>
        </nav>
      )}
    </div>
  );
}
