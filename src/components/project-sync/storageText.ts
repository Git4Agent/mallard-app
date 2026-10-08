import { backendText } from "../../i18n/backend";
import type {
  StorageFieldDescriptor,
  StorageProviderDescriptor,
  StorageSecretDescriptor,
  StorageSetupDoors,
  StorageSummaryField,
} from "../../types";

/**
 * A storage provider's words in the interface language. The descriptor's
 * English is the text; Chinese is looked up by the provider key and the
 * field, secret or fact id the backend sends beside each label. Called at
 * render, so a language switch redraws them.
 */

export function providerName(provider: Pick<StorageProviderDescriptor, "key" | "display_name">): string {
  return backendText(`storageProvider.${provider.key}.displayName`, provider.display_name);
}

export function providerDescription(provider: Pick<StorageProviderDescriptor, "key" | "description">): string {
  return backendText(`storageProvider.${provider.key}.description`, provider.description);
}

/** The provider a storage or a draft names: its descriptor's display name,
 *  carried as `provider_label`, by the storage's provider key. A storage
 *  this build has no adapter for carries its raw key instead, which stays. */
export function storageProviderName(storage: { provider: string; provider_label: string; available?: boolean }): string {
  if (storage.available === false) return storage.provider_label;
  return backendText(`storageProvider.${storage.provider}.displayName`, storage.provider_label);
}

export interface StorageInputText {
  label: string;
  placeholder?: string;
  help?: string;
}

function inputText(
  prefix: string,
  input: { label: string; placeholder?: string | null; help?: string | null },
): StorageInputText {
  return {
    label: backendText(`${prefix}.label`, input.label),
    placeholder: input.placeholder != null ? backendText(`${prefix}.placeholder`, input.placeholder) : undefined,
    help: input.help != null ? backendText(`${prefix}.help`, input.help) : undefined,
  };
}

export function fieldText(provider: string, field: StorageFieldDescriptor): StorageInputText {
  return inputText(`storageProvider.${provider}.field.${field.field_id}`, field);
}

export function secretText(provider: string, secret: StorageSecretDescriptor): StorageInputText {
  return inputText(`storageProvider.${provider}.secret.${secret.secret_id}`, secret);
}

/** A summary fact's label; its value is the backend's and stays as sent. */
export function summaryLabel(fact: Pick<StorageSummaryField, "field_id" | "label">): string {
  return backendText(`storageSummary.${fact.field_id}`, fact.label);
}

export function manualSetupHint(provider: Pick<StorageProviderDescriptor, "key" | "manual_setup_hint">): string | null {
  return provider.manual_setup_hint != null
    ? backendText(`storageProvider.${provider.key}.manualSetupHint`, provider.manual_setup_hint)
    : null;
}

/** The New-storage doors' sentences; the help link keeps its URL. */
export function setupDoorsText(provider: string, doors: StorageSetupDoors): StorageSetupDoors {
  const key = `storageProvider.${provider}.doors`;
  return {
    create_hint: backendText(`${key}.createHint`, doors.create_hint),
    existing_hint: backendText(`${key}.existingHint`, doors.existing_hint),
    ...(doors.existing_help
      ? { existing_help: { ...doors.existing_help, label: backendText(`${key}.existingHelpLabel`, doors.existing_help.label) } }
      : {}),
  };
}
