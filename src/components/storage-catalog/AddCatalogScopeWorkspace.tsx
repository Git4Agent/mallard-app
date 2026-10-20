import { homeDir } from "@tauri-apps/api/path";
import { useEffect, useRef, useState } from "react";
import { useTranslation } from "../../i18n";
import Icon from "../Icons";
import Notice from "../notice/Notice";
import ProviderMark from "../sync-review/ProviderMark";
import { errorMessage } from "../../storageError";
import { storageCatalogApi } from "./api";
import CatalogHomeStep from "./CatalogHomeStep";
import CatalogScopeSetupStep from "./CatalogScopeSetupStep";
import { useCloudRepositoryChoice } from "./CloudRepositoryChoice";
import {
  capabilityLabel,
  catalogCapabilityFor,
  catalogPresentation,
  catalogScopeLabel,
  sameCatalogCoordinate,
} from "./model";
import { SetupPill, SourceDoor } from "./setupFurniture";
import type { CatalogStorageOption } from "./StorageCatalogPage";
import type {
  CatalogCapabilityDescriptor,
  CatalogProfile,
  CatalogScopeDto,
} from "./types";

/** A Mallard Cloud account the door can add from. */
export interface CatalogCloudAccount {
  id: string;
  name: string;
  account_login: string;
}

/** Where an added item lands: its row, on its page. */
export interface CatalogCloudLanding {
  scope: CatalogScopeDto;
  storageId: string;
  resourceId: string;
  /** "Biscuit added from kai/Biscuit." */
  note: string;
}

interface Props {
  workflowEpoch: number;
  isWorkflowCurrent: (workflowEpoch: number) => boolean;
  busy: boolean;
  /** The Kind a door chose on the way in, as a choice key; null asks. */
  initialChoiceKey: string | null;
  /** The Catalog's rows: what a home already has, and where an item from
   *  Mallard Cloud lands. */
  scopes: readonly CatalogScopeDto[];
  /** Storages as the shell orders them for setup pages. */
  storages: readonly CatalogStorageOption[];
  cloudAccounts: readonly CatalogCloudAccount[];
  /** A repository the page was opened for (a link from the website): From
   *  Mallard Cloud opens on that account with the address looked up. */
  initialCloud?: CatalogCloudStart | null;
  onSetUpCloud: () => void;
  onClose: () => void;
  /** Finish on this computer: the row, the storage it was linked to, and the
   *  link's failure when the row was made but the link was not. */
  onCreated: (scope: CatalogScopeDto, storageId: string | null, linkError: string | null, workflowEpoch: number) => void;
  onAdded: (landing: CatalogCloudLanding, workflowEpoch: number) => void;
}

/** From Mallard Cloud, as New project's door is: the account, then the
 *  repository (shared with you, or by address or invitation code, its kind
 *  coming with it), then the Catalog row it lands in. Add saves the
 *  repository, makes the row and links it when either is missing, and opens
 *  the item's page. Nothing is pulled. */
function CloudSource({ accounts, scopes, profiles, capabilities, working, initial, onSetUpCloud, onChosen }: {
  accounts: readonly CatalogCloudAccount[];
  scopes: readonly CatalogScopeDto[];
  profiles: readonly CatalogProfile[];
  capabilities: readonly CatalogCapabilityDescriptor[];
  working: boolean;
  initial: CatalogCloudStart | null;
  onSetUpCloud: () => void;
  onChosen: (choice: CloudChoice | null) => void;
}) {
  const { t } = useTranslation("library");
  const [accountId, setAccountId] = useState<string | null>(() => (
    initial && accounts.some((candidate) => candidate.id === initial.accountId) ? initial.accountId : accounts[0]?.id ?? null
  ));
  const account = accounts.find((candidate) => candidate.id === accountId) ?? null;
  return (
    <>
      <section className="sync-setup-section">
        <div className="sync-card-heading"><div><strong>{t("bar.storage")}</strong></div></div>
        {accounts.length === 0 ? (
          <div className="r2-step-list">
            <div className="storage-found-empty" role="status">
              <Icon name="storage-mallard" size={24} />
              <strong>{t("add.noAccount")}</strong>
              <span className="storage-found-empty-verbs">
                <button type="button" className="btn btn-primary" onClick={onSetUpCloud} disabled={working}>
                  <Icon name="link" size={13} /> {t("add.setUpCloud")}
                </button>
              </span>
            </div>
          </div>
        ) : (
          <div className="setup-pill-row" role="radiogroup" aria-label={t("bar.storage")}>
            {accounts.map((candidate) => (
              <SetupPill key={candidate.id} active={candidate.id === accountId} disabled={working}
                mark={<Icon name="storage-mallard" size={12} />} label={`${candidate.name || "Mallard Cloud"} · ${candidate.account_login}`}
                onClick={() => { setAccountId(candidate.id); onChosen(null); }} />
            ))}
          </div>
        )}
      </section>
      {account && (
        <CloudRepositoryBands key={account.id} account={account} scopes={scopes} profiles={profiles}
          capabilities={capabilities} working={working} initialAddress={initial?.address ?? null} onChosen={onChosen} />
      )}
    </>
  );
}

/** Where From Mallard Cloud starts when the page was opened for one
 *  repository: the account that sees it, and its address. */
export interface CatalogCloudStart {
  accountId: string;
  address: string;
}

/** What the door's Add acts on. */
export interface CloudChoice {
  storageId: string;
  repository: { repository_id: string; resource_id: string; address: string; display_name: string };
  /** The row it lands in, or how to make one. */
  landing: { kind: "row"; scope: CatalogScopeDto } | { kind: "new"; capability: CatalogCapabilityDescriptor; profileId: string };
}

function CloudRepositoryBands({ account, scopes, profiles, capabilities, working, initialAddress, onChosen }: {
  account: CatalogCloudAccount;
  scopes: readonly CatalogScopeDto[];
  profiles: readonly CatalogProfile[];
  capabilities: readonly CatalogCapabilityDescriptor[];
  working: boolean;
  initialAddress: string | null;
  onChosen: (choice: CloudChoice | null) => void;
}) {
  const { t } = useTranslation("library");
  const choice = useCloudRepositoryChoice({
    storageId: account.id,
    scopeId: null,
    accountLogin: account.account_login,
    target: { kind: "door" },
    working,
    capabilities,
    list: () => storageCatalogApi.listCloudShared(account.id),
    listKey: account.id,
    initialAddress,
  });
  const chosen = choice.chosen;
  const capability = chosen ? catalogCapabilityFor(capabilities, chosen) : null;
  const rowsOfKind = chosen ? scopes.filter((scope) => sameCatalogCoordinate(scope.scope, chosen)) : [];
  const providerProfiles = capability ? profiles.filter((profile) => profile.provider === capability.provider) : [];
  const [rowId, setRowId] = useState<string | null>(null);
  const [profileId, setProfileId] = useState<string | null>(null);
  const row = rowsOfKind.find((scope) => scope.scope.scope_id === rowId) ?? rowsOfKind[0] ?? null;
  const profile = providerProfiles.find((candidate) => candidate.profile_id === profileId) ?? providerProfiles[0] ?? null;

  const repositoryId = chosen?.repository_id ?? null;
  const landingKey = row ? `row:${row.scope.scope_id}` : profile ? `profile:${profile.profile_id}` : null;
  useEffect(() => {
    if (!chosen || !capability || !landingKey) {
      onChosen(null);
      return;
    }
    onChosen({
      storageId: account.id,
      repository: chosen,
      landing: row ? { kind: "row", scope: row } : { kind: "new", capability, profileId: profile!.profile_id },
    });
  }, [repositoryId, landingKey]);

  const glyph = catalogPresentation(capability).icon;
  return (
    <>
      <section className="sync-setup-section repo-connect">
        <div className="sync-card-heading"><div><strong>{t("add.repository")}</strong></div></div>
        {choice.field}
        {choice.found}
        {choice.list}
      </section>
      <section className="sync-setup-section">
        <div className="sync-card-heading"><div><strong>{t("bar.row")}</strong></div></div>
        {!chosen || !capability ? (
          <small className="sync-setup-hint">{t("add.chooseRepository")}</small>
        ) : rowsOfKind.length > 0 ? (
          <div className="setup-pill-row" role="radiogroup" aria-label={t("bar.row")}>
            {rowsOfKind.map((scope) => (
              <SetupPill key={scope.scope.scope_id} active={scope.scope.scope_id === row?.scope.scope_id} disabled={working}
                mark={<Icon name={glyph} size={12} />} label={catalogScopeLabel({ ...scope, capability })}
                onClick={() => setRowId(scope.scope.scope_id)} />
            ))}
          </div>
        ) : providerProfiles.length > 0 ? (
          <>
            <div className="setup-pill-row" role="radiogroup" aria-label={t("add.agentProfile")}>
              {providerProfiles.map((candidate) => (
                <SetupPill key={candidate.profile_id} active={candidate.profile_id === profile?.profile_id} disabled={working}
                  mark={<ProviderMark provider={candidate.provider} displayName={candidate.provider_label} size={12} />}
                  label={candidate.display_name} onClick={() => setProfileId(candidate.profile_id)} />
              ))}
            </div>
            <small className="sync-setup-hint">{t("add.creates", { label: capabilityLabel(capability), profile: profile?.display_name })}</small>
          </>
        ) : (
          <small className="sync-setup-hint">{t("add.homeFirst", { provider: capability.provider_label })}</small>
        )}
      </section>
    </>
  );
}

/** Where the local door stands: choosing the home, or setting it up. */
type Stage = { kind: "home" } | { kind: "setup"; profile: CatalogProfile };

/**
 * Add Catalog resource: New project's two doors. On this computer is New
 * project's two steps — the agent home, chosen as the project chooses its
 * agents, then that home's setup page: one kind, the storage, Finish. From
 * Mallard Cloud is a repository shared with you, which brings its kind with
 * it. Either way one row is added per pass.
 */
export default function AddCatalogScopeWorkspace({
  workflowEpoch,
  isWorkflowCurrent,
  busy,
  initialChoiceKey,
  scopes,
  storages,
  cloudAccounts,
  initialCloud = null,
  onSetUpCloud,
  onClose,
  onCreated,
  onAdded,
}: Props) {
  const { t } = useTranslation(["library", "common"]);
  const [source, setSource] = useState<"local" | "cloud">(initialCloud ? "cloud" : "local");
  const [stage, setStage] = useState<Stage>({ kind: "home" });
  const [cloudChoice, setCloudChoice] = useState<CloudChoice | null>(null);
  /* A row Add made before its repository could be saved: a retry uses it. */
  const [madeScopes, setMadeScopes] = useState<CatalogScopeDto[]>([]);
  const [profiles, setProfiles] = useState<CatalogProfile[]>([]);
  const [capabilities, setCapabilities] = useState<CatalogCapabilityDescriptor[]>([]);
  const [homePath, setHomePath] = useState<string | null>(null);
  const [submitting, setSubmitting] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const mountedRef = useRef(true);
  useEffect(() => {
    mountedRef.current = true;
    return () => {
      mountedRef.current = false;
    };
  }, []);
  const workflowIsActive = () => mountedRef.current && isWorkflowCurrent(workflowEpoch);

  useEffect(() => {
    let cancelled = false;
    void (async () => {
      try {
        const [nextProfiles, nextCapabilities] = await Promise.all([
          storageCatalogApi.listProfiles(),
          storageCatalogApi.listCapabilities(),
        ]);
        if (cancelled || !mountedRef.current) return;
        setProfiles(nextProfiles);
        setCapabilities(nextCapabilities);
      } catch (reason) {
        if (!cancelled && mountedRef.current) setError(errorMessage(reason));
      }
    })();
    return () => {
      cancelled = true;
    };
  }, []);

  useEffect(() => {
    let cancelled = false;
    void homeDir()
      .then((resolved) => {
        if (!cancelled) setHomePath(resolved.replace(/\/+$/, ""));
      })
      .catch(() => {
        if (!cancelled) setHomePath(null);
      });
    return () => {
      cancelled = true;
    };
  }, []);

  const working = busy || submitting;

  /* Add from Mallard Cloud: make the row and link Mallard Cloud when either
     is missing, then save the repository. Each step is safe to repeat. */
  const addFromCloud = async () => {
    if (!cloudChoice) return;
    setSubmitting(true);
    setError(null);
    try {
      const { landing, repository, storageId } = cloudChoice;
      let scope = landing.kind === "row" ? landing.scope : madeScopes.find((made) => (
        made.profile.profile_id === landing.profileId && sameCatalogCoordinate(made.scope, landing.capability)
      )) ?? null;
      if (!scope && landing.kind === "new") {
        scope = await storageCatalogApi.createScope(landing.capability, landing.profileId);
        if (!workflowIsActive()) return;
        const made = scope;
        setMadeScopes((current) => [...current, made]);
      }
      if (!scope) return;
      const link = await storageCatalogApi.linkScopeStorage(scope.scope.scope_id, storageId);
      await storageCatalogApi.connectCloudRepository(scope.scope.scope_id, storageId, repository.resource_id, repository.repository_id, link.revision);
      if (!workflowIsActive()) return;
      onAdded({
        scope,
        storageId,
        resourceId: repository.resource_id,
        note: t("cloud.added", { name: repository.display_name, address: repository.address }),
      }, workflowEpoch);
    } catch (reason) {
      if (workflowIsActive()) setError(errorMessage(reason));
    } finally {
      if (workflowIsActive()) setSubmitting(false);
    }
  };

  if (stage.kind === "setup") {
    return (
      <main className="sync-main sync-project-links-page sync-project-setup-page">
        <section className="profile-links-section">
          <CatalogScopeSetupStep
            key={stage.profile.profile_id}
            profile={stage.profile}
            capabilities={capabilities}
            scopes={[...scopes, ...madeScopes]}
            storages={storages}
            initialChoiceKey={initialChoiceKey}
            busy={busy}
            isActive={workflowIsActive}
            onBack={() => setStage({ kind: "home" })}
            onClose={onClose}
            onFinished={(scope, storageId, linkError) => onCreated(scope, storageId, linkError, workflowEpoch)}
          />
        </section>
      </main>
    );
  }

  const doors = (
    <div className="storage-mode-doors" role="radiogroup" aria-label={t("add.source")}>
      <SourceDoor icon="computer" title={t("add.local")} detail={t("add.localDetail")}
        checked={source === "local"} disabled={submitting} onChoose={() => { setSource("local"); setError(null); }} />
      <SourceDoor icon="storage-mallard" title={t("add.cloud")} detail={t("add.cloudDetail")}
        checked={source === "cloud"} disabled={submitting} onChoose={() => { setSource("cloud"); setError(null); }} />
    </div>
  );
  const errorNotice = error && (
    /* The page's failure, not a field's: the profile save, the scope creation
       or the mount listing that did not happen. It announces assertively. */
    <Notice severity="danger" timing="blocking" className="sync-setup-system-error">
      {error}
    </Notice>
  );

  return (
    <main className="sync-main sync-project-links-page sync-project-setup-page">
      <section className="profile-links-section">
        <section
          className="sync-inline-new-project sync-setup-workspace"
          role="region"
          aria-labelledby="catalog-add-scope-title"
        >
          <header className="sync-modal-header">
            <div>
              <h1 id="catalog-add-scope-title">{t("add.title")}</h1>
            </div>
            <button
              type="button"
              className="btn btn-ghost"
              onClick={onClose}
              disabled={submitting}
              aria-label={t("add.close")}
            >
              <Icon name="x" size={17} />
            </button>
          </header>

          {source === "cloud" ? (
            <>
              <div className="sync-modal-body">
                {doors}
                <CloudSource accounts={cloudAccounts} scopes={[...scopes, ...madeScopes]} profiles={profiles}
                  capabilities={capabilities} working={working} initial={initialCloud} onSetUpCloud={onSetUpCloud} onChosen={setCloudChoice} />
                {errorNotice}
              </div>
              <footer className="sync-modal-footer sync-setup-footer">
                {/* An added item keeps its repository; said before Add. */}
                {cloudChoice && <span className="sync-setup-save-state">{t("add.fixed")}</span>}
                <div>
                  <button type="button" className="btn btn-ghost" onClick={onClose} disabled={submitting}>
                    {t("common:cancel")}
                  </button>
                  <button
                    type="button"
                    className="btn btn-primary"
                    disabled={working || !cloudChoice}
                    onClick={() => void addFromCloud()}
                  >
                    <Icon name="plus" size={13} />
                    {submitting ? t("add.adding") : t("add.add")}
                  </button>
                </div>
              </footer>
            </>
          ) : (
            <CatalogHomeStep
              doors={doors}
              error={errorNotice}
              profiles={profiles}
              capabilities={capabilities}
              homePath={homePath}
              working={busy}
              isActive={workflowIsActive}
              onError={setError}
              onClose={onClose}
              onChosen={(profile) => {
                setError(null);
                setProfiles((current) => (
                  current.some((candidate) => candidate.profile_id === profile.profile_id)
                    ? current.map((candidate) => (candidate.profile_id === profile.profile_id ? profile : candidate))
                    : [...current, profile]
                ));
                setStage({ kind: "setup", profile });
              }}
            />
          )}
        </section>
      </section>
    </main>
  );
}
