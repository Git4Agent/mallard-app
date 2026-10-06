import type { ReactNode } from "react";
import Icon from "../Icons";
import ThreadStatusIcon from "./ThreadStatusIcon";

/** The parts a status splits a project list into, in the path bar's own
 *  terms: its two verbs, its attention chip, and what is left. */
export type StatusScope = "push" | "pull" | "attention" | "synced" | "unknown";

/** The scopes in the order the bar shows them: work first, then what is
 *  settled, then what was never judged. Each wears the glyph its rows do;
 *  Attention wears the path bar chip's. Both faces of a project read this
 *  one list, so Sessions and Files say the same five words. */
export const STATUS_SCOPES: readonly { id: StatusScope; label: string; mark: ReactNode }[] = [
  { id: "push", label: "To push", mark: <ThreadStatusIcon status="local-outstanding" size={15} /> },
  { id: "pull", label: "To pull", mark: <ThreadStatusIcon status="cloud-outstanding" size={15} /> },
  { id: "attention", label: "Attention", mark: <Icon name="alert-triangle" size={14} /> },
  { id: "synced", label: "Up to date", mark: <ThreadStatusIcon status="synced" size={15} /> },
  { id: "unknown", label: "Not compared", mark: <ThreadStatusIcon status="unknown" size={15} /> },
];
