import { StrictMode } from "react";
import { createRoot } from "react-dom/client";
import Icon from "./components/Icons";
import SiblingRepoNotice from "./components/project-sync/SiblingRepoNotice";
import type { BundleSnapshotSummary } from "./types";
import "./App.css";
import "./RetroTheme.css";

/**
 * Repo bundle convergence — specimen (`docs/PLAN_REPO_BUNDLE_CONVERGENCE.md`).
 *
 * Three surfaces, drawn with the shipped classes and components:
 * - The setup Repository section with an empty storage: the same option
 *   card the match list uses, alone and selected, carrying the creation
 *   statement — no more floating sentence.
 * - The same section when the storage holds a registration claim: the
 *   fingerprint match pre-selected, "New repository" as the alternative.
 * - The links-workspace sibling banner, all three states, rendered by the
 *   shipped SiblingRepoNotice component.
 *
 * Not part of the app bundle: the build has one entry, and this file is
 * reachable only from the dev server.
 */

const claim: BundleSnapshotSummary = {
  storage_id: "storage-1",
  bundle_id: "19e9afda78c650335f05301a3c0d09e9",
  display_name: "HorrorGame",
  revision: 1,
  updated_at: Math.floor(Date.now() / 1000) - 3600,
  resource_count: 0,
  registration_only: true,
  repository_fingerprint: "a".repeat(64),
};

const pushed: BundleSnapshotSummary = {
  ...claim,
  revision: 2,
  resource_count: 2,
  registration_only: false,
};

function RepositoryCard({
  active,
  title,
  subtitle,
  badge,
  meta,
  separate,
}: {
  active: boolean;
  title: string;
  subtitle: string;
  badge?: string;
  meta?: string;
  separate?: boolean;
}) {
  return (
    <button
      type="button"
      role="radio"
      aria-checked={active}
      className={`sync-bundle-match-option${separate ? " separate" : ""}${active ? " active" : ""}`}
    >
      <span className="sync-bundle-radio"><span /></span>
      <span className="sync-bundle-match-copy">
        <strong>{title}{badge && <small className="sync-bundle-recommended">{badge}</small>}</strong>
        <span>{subtitle}</span>
      </span>
      {meta && <span className="sync-bundle-match-meta">{meta}</span>}
    </button>
  );
}

function RepositorySection({ children, label }: { children: React.ReactNode; label: string }) {
  return (
    <section className="sync-setup-section" aria-label={label}>
      <div className="sync-card-heading">
        <div><strong>Repository</strong></div>
      </div>
      <div className="sync-bundle-match-options" role="radiogroup" aria-label={label}>
        {children}
      </div>
    </section>
  );
}

function Specimen({ title, children }: { title: string; children: React.ReactNode }) {
  return (
    <section style={{ marginBottom: "2.5rem" }}>
      <h2 style={{ font: "600 0.8rem/1.4 var(--font-pixel, monospace)", opacity: 0.6, margin: "0 0 0.75rem" }}>
        <Icon name="git-compare-arrows" size={13} /> {title}
      </h2>
      {children}
    </section>
  );
}

function App() {
  return (
    <div className="sync-inline-new-project sync-setup-workspace" style={{ maxWidth: 760, margin: "2rem auto", padding: "0 1.5rem" }}>
      <Specimen title="Setup · Repository — empty storage (creation statement as the selected card)">
        <RepositorySection label="Repo identity for this project (empty storage)">
          <RepositoryCard
            active
            separate
            title="New repository"
            subtitle="Creates “HorrorGame” in Local storage 1 — machines joining this repository later will find it there"
          />
        </RepositorySection>
      </Specimen>

      <Specimen title="Setup · Repository — a registration claim exists (Git match pre-selected)">
        <RepositorySection label="Repo identity for this project (claim found)">
          <RepositoryCard
            active={false}
            separate
            title="New repository"
            subtitle="Keep this project separate"
          />
          <RepositoryCard
            active
            title="HorrorGame"
            badge="Git match"
            subtitle="19e9afda78c650335f05301a3c0d09e9"
            meta="1h ago"
          />
        </RepositorySection>
      </Specimen>

      <Specimen title="Links workspace · sibling banner — content sibling">
        <SiblingRepoNotice
          status={{ state: "sibling_found", matches: [pushed] }}
          busy={false}
          onConnect={() => undefined}
        />
      </Specimen>

      <Specimen title="Links workspace · sibling banner — registration sibling">
        <SiblingRepoNotice
          status={{ state: "sibling_found", matches: [claim] }}
          busy={false}
          onConnect={() => undefined}
        />
      </Specimen>

      <Specimen title="Links workspace · sibling banner — concurrent-claim twin">
        <SiblingRepoNotice
          status={{ state: "registration_twin", survivor: claim }}
          busy={false}
          onConnect={() => undefined}
        />
      </Specimen>
    </div>
  );
}

createRoot(document.getElementById("root") as HTMLElement).render(
  <StrictMode>
    <App />
  </StrictMode>,
);
