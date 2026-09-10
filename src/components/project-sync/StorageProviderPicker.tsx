import type { StorageProviderDescriptor, StorageProviderKey } from "../../types";
import Icon, { iconNameFromHint } from "../Icons";
import SyncPathBar, { type PathBarMenuGroup } from "../sync-review/SyncPathBar";

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
  return (
    <>
      <SyncPathBar
        crumbs={[
          {
            id: "storages",
            label: "Storages",
            icon: "storage",
            tone: "root",
            title: "All storages",
            ariaLabel: "All storages",
            onActivate: onCancel,
            disabled: busy,
          },
          { id: "storage", label: "New storage" },
        ]}
        overflow={appActions}
        overflowLabel="App actions"
        ariaLabel="New storage path"
      />
      <main className="sync-main sync-project-links-page sync-landing-page">
        {providers.length === 0 ? (
          <section className="landing-picker landing-picker-empty" aria-labelledby="landing-picker-title">
            <span className="landing-picker-mark" aria-hidden="true">
              <img src="/mallard-logo-duck.svg" alt="" />
            </span>
            <h1 id="landing-picker-title" className="settings-section-title">No storage providers in this build</h1>
            <button type="button" className="btn" onClick={onCancel} disabled={busy}>Back to Storages</button>
          </section>
        ) : (
          <section className="landing-picker" aria-labelledby="landing-picker-title">
            <div className="landing-picker-head">
              <h1 id="landing-picker-title" className="settings-section-title">Choose a storage type</h1>
              <button type="button" className="btn btn-ghost" onClick={onCancel} disabled={busy}>Cancel</button>
            </div>
            <ul className="landing-picker-grid">
              {providers.map((provider) => (
                <li key={provider.key}>
                  <button
                    type="button"
                    className="landing-picker-card"
                    aria-label={`Set up ${provider.display_name} storage`}
                    disabled={busy}
                    onClick={() => onChoose(provider.key)}
                  >
                    <span className="landing-picker-card-title">
                      <Icon name={iconNameFromHint(provider.icon, "drive")} size={17} />
                      <strong>{provider.display_name}</strong>
                    </span>
                    <span className="landing-picker-card-detail">{provider.description}</span>
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
