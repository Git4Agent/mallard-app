import { useEffect, useRef, useState } from "react";
import { useTranslation } from "../../i18n";
import { backendText } from "../../i18n/backend";
import Icon, { storageIconName } from "../Icons";
import Notice, { FieldMessage } from "../notice/Notice";
import { errorMessage } from "../../storageError";
import { storageCatalogApi } from "./api";
import {
  capabilityLabel,
  catalogDefaultKind,
  catalogDefaultStorage,
  catalogKindCardFoot,
  catalogKindCards,
  catalogPresentation,
  catalogStorageHint,
} from "./model";
import { SetupPill, attentionBadge } from "./setupFurniture";
import type { CatalogStorageOption } from "./StorageCatalogPage";
import type {
  CatalogCapabilityDescriptor,
  CatalogProfile,
  CatalogProfileInspection,
  CatalogScopeDto,
} from "./types";

interface Props {
  profile: CatalogProfile;
  capabilities: readonly CatalogCapabilityDescriptor[];
  /** The Catalog's rows: a kind this home already has is not offered again. */
  scopes: readonly CatalogScopeDto[];
  /** Storages as the shell orders them: Mallard Cloud, the other clouds,
   *  then local folders. */
  storages: readonly CatalogStorageOption[];
  /** The Kind a door chose on the way in, as a choice key; null asks. */
  initialChoiceKey: string | null;
  busy: boolean;
  isActive: () => boolean;
  onBack: () => void;
  onClose: () => void;
  /** Finish: the row, the storage it was linked to, and the link's failure
   *  when the row was made but the link was not. */
  onFinished: (scope: CatalogScopeDto, storageId: string | null, linkError: string | null) => void;
}

/**
 * Step two of Add Catalog resource: "Set up Default Codex". The home, one
 * kind among those it holds — the shipped Kind cards, with what the home
 * holds on the foot line — and the storage, then Finish, which makes
 * exactly one row and links it. Nothing is read or copied until Push or
 * Pull on the row's page.
 */
export default function CatalogScopeSetupStep({
  profile,
  capabilities,
  scopes,
  storages,
  initialChoiceKey,
  busy,
  isActive,
  onBack,
  onClose,
  onFinished,
}: Props) {
  const { t } = useTranslation(["library", "common"]);
  const [inspection, setInspection] = useState<CatalogProfileInspection | null>(null);
  const [inspectError, setInspectError] = useState<string | null>(null);
  const [name, setName] = useState(profile.display_name);
  const [picked, setPicked] = useState<string | null>(null);
  const pickedByUser = useRef(false);
  const [storage, setStorage] = useState<string | null>(() => catalogDefaultStorage(storages, null));
  const storageByUser = useRef(false);
  const [submitting, setSubmitting] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const cards = catalogKindCards(capabilities, profile, scopes, inspection?.holdings ?? null);

  /* What the home holds arrives after the page opens; until the user has
     chosen, the kind and the storage follow it. */
  useEffect(() => {
    let cancelled = false;
    void storageCatalogApi.inspectProfile(profile.profile_id)
      .then((next) => {
        if (cancelled || !isActive()) return;
        setInspection(next);
      })
      .catch((reason) => {
        if (!cancelled && isActive()) setInspectError(errorMessage(reason));
      });
    return () => { cancelled = true; };
  }, [profile.profile_id]);
  useEffect(() => {
    if (!pickedByUser.current) setPicked(catalogDefaultKind(cards, initialChoiceKey));
    if (!storageByUser.current) setStorage(catalogDefaultStorage(storages, inspection?.last_used_storage_id));
  }, [inspection, capabilities.length, scopes.length, storages.length]);

  const card = cards.find((candidate) => candidate.key === picked) ?? null;
  /* The band asks for a kind only once what the home holds is known — or
     known to be unknowable — so an opening default never flashes a fault. */
  const settled = inspection !== null || inspectError !== null;
  const chosenStorage = storages.find((candidate) => candidate.id === storage) ?? null;
  const working = busy || submitting;
  const shownName = name.trim() || profile.display_name;

  /* Finish makes one row; a renamed home is renamed first; the storage is
     linked last, and a link that fails leaves the row standing — its page
     is where the storage menu retries. */
  const finish = async () => {
    if (!card || working) return;
    setSubmitting(true);
    setError(null);
    try {
      if (name.trim() && name.trim() !== profile.display_name) {
        await storageCatalogApi.renameProfile(profile.profile_id, name.trim(), profile.revision);
        if (!isActive()) return;
      }
      const created = await storageCatalogApi.createScope(card.capability, profile.profile_id);
      if (!isActive()) return;
      if (!storage) {
        onFinished(created, null, null);
        return;
      }
      try {
        await storageCatalogApi.linkScopeStorage(created.scope.scope_id, storage);
        if (!isActive()) return;
        onFinished(created, storage, null);
      } catch (reason) {
        if (isActive()) onFinished(created, null, errorMessage(reason));
      }
    } catch (reason) {
      if (isActive()) setError(errorMessage(reason));
    } finally {
      if (isActive()) setSubmitting(false);
    }
  };

  const storageHint = catalogStorageHint(storages, storage, inspection?.last_used_storage_id);
  return (
    <section className="sync-inline-new-project sync-setup-workspace" role="region" aria-labelledby="catalog-setup-title">
      <header className="sync-modal-header">
        <div>
          <h1 id="catalog-setup-title">{t("home.setUpNamed", { name: shownName })}</h1>
        </div>
        <button type="button" className="btn btn-ghost" onClick={onClose} disabled={submitting} aria-label={t("setup.close")}>
          <Icon name="x" size={17} />
        </button>
      </header>

      <div className="sync-modal-body">
        <section className="sync-setup-section">
          <div className="sync-card-heading"><div><strong>{t("setup.home")}</strong></div></div>
          <div className="sync-field-grid sync-setup-project-fields">
            <label>
              <span>{t("setup.name")}</span>
              <input value={name} disabled={working} onChange={(event) => setName(event.target.value)} />
            </label>
            <label>
              <span>{t("setup.folder")}</span>
              <input value={profile.configured_home} readOnly />
            </label>
          </div>
        </section>

        <section className="sync-setup-section">
          <div className="sync-card-heading">
            <div><strong>{t("setup.resource")}</strong></div>
            {attentionBadge(settled && card === null)}
          </div>
          {/* The landing door, filled: the shipped Kind card, with what the
              home holds where the door wears its marks. */}
          <div className="setup-choice-grid" role="radiogroup" aria-label={t("setup.kind")}>
            {cards.map((candidate) => {
              const active = picked === candidate.key;
              const added = candidate.existingScopeId !== null;
              return (
                <button
                  key={candidate.key}
                  type="button"
                  role="radio"
                  aria-checked={active}
                  className={`setup-choice-card${active ? " active" : ""}`}
                  disabled={working || added}
                  title={candidate.holding?.roots.join(", ") || undefined}
                  onClick={() => {
                    pickedByUser.current = true;
                    setPicked(candidate.key);
                  }}
                >
                  <span className="setup-choice-card-head">
                    <span className="setup-choice-mark" aria-hidden="true">
                      <Icon name={catalogPresentation(candidate.capability).icon} size={24} />
                    </span>
                    <strong>
                      {capabilityLabel(candidate.capability)}
                      {added && <small className="sync-bundle-recommended">{t("setup.added")}</small>}
                    </strong>
                    <span className="setup-choice-card-check" aria-hidden="true">
                      {active && <Icon name="check" size={12} />}
                    </span>
                  </span>
                  <span className="setup-choice-card-foot">{catalogKindCardFoot(candidate)}</span>
                </button>
              );
            })}
          </div>
          {settled && card === null && cards.length > 0 && <FieldMessage id="catalog-setup-error-resource">{t("setup.chooseKind")}</FieldMessage>}
          {inspectError && <small className="sync-setup-hint">{t("setup.inspectFailed", { error: inspectError })}</small>}
        </section>

        <section className="sync-setup-section">
          <div className="sync-card-heading"><div><strong>{t("bar.storage")}</strong></div></div>
          <div className="setup-pill-row" role="radiogroup" aria-label={t("bar.storage")}>
            <SetupPill
              active={storage === null}
              disabled={working}
              mark={<Icon name="computer" size={12} />}
              label={t("setup.noneYet")}
              title={t("setup.noneTitle")}
              onClick={() => { storageByUser.current = true; setStorage(null); }}
            />
            {storages.map((candidate) => (
              <SetupPill
                key={candidate.id}
                active={storage === candidate.id}
                disabled={working}
                mark={<Icon name={storageIconName(candidate.provider, candidate.icon)} size={12} />}
                label={candidate.name || t("bar.unnamed")}
                title={`${candidate.name || t("bar.unnamed")} · ${backendText(`storageProvider.${candidate.provider}.displayName`, candidate.provider_label)}`}
                onClick={() => { storageByUser.current = true; setStorage(candidate.id); }}
              />
            ))}
          </div>
          {storageHint && <small className="sync-setup-hint">{storageHint}</small>}
        </section>

        {error && (
          <Notice severity="danger" timing="blocking" className="sync-setup-system-error">
            {error}
          </Notice>
        )}
      </div>

      <footer className="sync-modal-footer sync-setup-footer">
        <button type="button" className="btn btn-ghost" onClick={onBack} disabled={submitting}>
          <Icon name="chevron-left" size={13} /> {t("common:back")}
        </button>
        {card && <span className="sync-setup-save-state">{capabilityLabel(card.capability)} · {shownName}</span>}
        <div>
          <button type="button" className="btn btn-ghost" onClick={onClose} disabled={submitting}>
            {t("common:cancel")}
          </button>
          <button type="button" className="btn btn-primary" disabled={!card || working} onClick={() => void finish()}>
            {submitting ? t("setup.finishing") : chosenStorage ? t("setup.finish") : t("setup.finishLocally")}
          </button>
        </div>
      </footer>
    </section>
  );
}
