import type { ReactNode } from "react";
import { t } from "../../i18n";
import Icon from "../Icons";
import ThreadStatusIcon from "./ThreadStatusIcon";

/** The parts a status splits a project list into, in the path bar's own
 *  terms: its two verbs, its attention chip, and what is left. */
export type StatusScope = "push" | "pull" | "attention" | "synced" | "unknown";

/** One scope with its word read when it is read: a list spreads the scope
 *  into its bar during render, so the label is the interface language's at
 *  that moment, not the one this module loaded in. */
function scope(id: StatusScope, mark: ReactNode): { id: StatusScope; readonly label: string; mark: ReactNode } {
  return {
    id,
    mark,
    get label() {
      return t(`syncReview:scopes.${id}`);
    },
  };
}

/** The scopes in the order the bar shows them: work first, then what is
 *  settled, then what was never judged. Each wears the glyph its rows do;
 *  Attention wears the path bar chip's. Both faces of a project read this
 *  one list, so Sessions and Files say the same five words. */
export const STATUS_SCOPES: readonly { id: StatusScope; readonly label: string; mark: ReactNode }[] = [
  scope("push", <ThreadStatusIcon status="local-outstanding" size={15} />),
  scope("pull", <ThreadStatusIcon status="cloud-outstanding" size={15} />),
  scope("attention", <Icon name="alert-triangle" size={14} />),
  scope("synced", <ThreadStatusIcon status="synced" size={15} />),
  scope("unknown", <ThreadStatusIcon status="unknown" size={15} />),
];
