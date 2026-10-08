import type { StorageProviderDescriptor, StorageProviderKey } from "../../types";
import Icon, { storageIconName } from "../Icons";
import SyncPathBar, { type PathBarMenuGroup } from "../sync-review/SyncPathBar";
import { useTranslation } from "../../i18n";
import { providerDescription, providerName } from "./storageText";

/**
 * The first step of Add storage: which provider the new storage uses. A
 * draft is bound to its provider from the moment it exists, so the choice
 * comes before the form rather than as a switch inside it. The cards are
 * the backend's descriptors in the backend's order; the page holds no
 * provider list of its own.
 */
interface Props {
  providers: StorageProviderDescriptor[];
  busy: boolean;
  /** Theme and app-wide commands; the bar here is the window's only chrome. */
  appActions: PathBarMenuGroup[];
  onChoose: (provider: StorageProviderKey) => void;
  onCancel: () => void;
}

export default function StorageProviderPicker({ providers, busy, appActions, onChoose, onCancel }: Props) {
  const { t } = useTranslation(["storage", "common"]);
  // Feishu is temporarily hidden from storage setup.
  const visibleProviders = providers.filter((provider) => provider.key !== "feishu");
  return (
    <>
      <SyncPathBar
        crumbs={[
          {
            id: "storages",
            label: t("shared.storages"),
            icon: "storage",
            tone: "root",
            title: t("picker.allStorages"),
            ariaLabel: t("picker.allStorages"),
            onActivate: onCancel,
            disabled: busy,
          },
          { id: "storage", label: t("picker.newStorage") },
        ]}
        overflow={appActions}
        overflowLabel={t("shared.appActions")}
        ariaLabel={t("picker.pathLabel")}
      />
      <main className="sync-main sync-project-links-page sync-landing-page">
        {visibleProviders.length === 0 ? (
          <section className="landing-picker landing-picker-empty" aria-labelledby="landing-picker-title">
            <span className="landing-picker-mark" aria-hidden="true">
              <img src="/mallard-logo-duck.svg" alt="" />
            </span>
            <h1 id="landing-picker-title" className="settings-section-title">{t("picker.empty")}</h1>
            <button type="button" className="btn" onClick={onCancel} disabled={busy}>{t("picker.back")}</button>
          </section>
        ) : (
          <section className="landing-picker" aria-labelledby="landing-picker-title">
            <div className="landing-picker-head">
              <h1 id="landing-picker-title" className="settings-section-title">{t("picker.title")}</h1>
              <button type="button" className="btn btn-ghost" onClick={onCancel} disabled={busy}>{t("common:cancel")}</button>
            </div>
            <ul className="landing-picker-grid">
              {visibleProviders.map((provider) => (
                <li key={provider.key}>
                  <button
                    type="button"
                    className="landing-picker-card"
                    aria-label={t("picker.setUp", { provider: providerName(provider) })}
                    disabled={busy}
                    onClick={() => onChoose(provider.key)}
                  >
                    <span className="landing-picker-card-title">
                      <Icon name={storageIconName(provider.key, provider.icon)} size={17} />
                      <strong>{providerName(provider)}</strong>
                    </span>
                    <span className="landing-picker-card-detail">{providerDescription(provider)}</span>
                  </button>
                </li>
              ))}
            </ul>
          </section>
        )}
      </main>
    </>
  );
}
