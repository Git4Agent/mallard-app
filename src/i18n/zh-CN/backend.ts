import { zhBackendCodes } from "./backend-codes";
import { zhBackendLabels } from "./backend-labels";

/**
 * Chinese for the fixed labels the backend sends beside a stable key; read
 * through `backendText` in `src/i18n/backend.ts`. English is the backend's
 * own text and is not repeated here. Placeholders are `{{name}}`.
 *
 * Two parts: `backend-labels.ts` for labels whose key the backend already
 * sends (provider fields, summary labels, resource kinds, capabilities,
 * agent and session actions, blockers), `backend-codes.ts` for labels whose
 * code the backend sends since D7 (discovery facts, count explanations,
 * warnings, progress, readiness, after-Pull offers).
 */
export const zhBackend = { ...zhBackendLabels, ...zhBackendCodes } as const;
