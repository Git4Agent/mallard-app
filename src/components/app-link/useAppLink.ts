import { useEffect, useRef, useState } from "react";
import { errorMessage } from "../../storageError";
import type { AppLinkArrival } from "../../types";
import { projectSyncApi } from "../project-sync/api";
import { storageCatalogApi } from "../storage-catalog/api";
import type { CatalogCloudPlace } from "../storage-catalog/types";
import type { AppLinkSheetState } from "./AppLinkSheet";
import {
  catalogSeers,
  INVALID_LINK,
  linkVerdict,
  listingVerdict,
  unanswered,
  type LinkContext,
  type LinkDestination,
  type LinkVerdict,
} from "./appLink";

/** The link the shell holds: one at a time, the newest. Nothing about it is
 *  saved; it does not outlive the window. */
export interface HeldLink {
  link: AppLinkArrival;
  /** `waiting`: a line under the bar, with Open. `open`: its sheet is up. */
  phase: "waiting" | "open";
  /** Set aside while Mallard Cloud is signed in to; a new account opens it. */
  awaitsAccount: boolean;
  /** This arrival, and no other: its sheet, and what may close it. */
  id: number;
  /** This asking of the service; a reopening or Try again is another. */
  round: number;
}

/** This computer's half of a link's answer, read when it is needed. */
export type ShellLinkContext = Omit<LinkContext, "places"> & {
  /** `serviceHost` of each Mallard Cloud storage. */
  cloudHosts: readonly string[];
};

/**
 * Takes `mallard://` links from the backend once the shell has loaded and
 * holds the newest. A repository's link, and a session's, opens a sheet and
 * is answered: what each account on its host sees at the repository, joined
 * with what is on this computer; the shell follows the verdict when the
 * sheet's verb is pressed. The service is not asked about a session: its
 * Pull review is what reads the storage. The listing's
 * link asks nothing: with an account on its host it goes straight to its
 * page through `go`, and only without one does it open the sheet. Either
 * kind waits under the bar while the shell is in the middle of something.
 */
export function useAppLink({ ready, waits, context, go }: {
  ready: boolean;
  /** Whether a link arriving now must wait under the bar (`linkWaits`). */
  waits: () => boolean;
  context: () => ShellLinkContext;
  /** Navigate for a link that needs no sheet. */
  go: (destination: LinkDestination) => void;
}) {
  const [held, setHeld] = useState<HeldLink | null>(null);
  const [answered, setAnswered] = useState<{ round: number; verdict: LinkVerdict } | null>(null);
  const sequence = useRef(0);
  const heldRef = useRef(held);
  heldRef.current = held;
  const waitsRef = useRef(waits);
  waitsRef.current = waits;
  const contextRef = useRef(context);
  contextRef.current = context;
  const goRef = useRef(go);
  goRef.current = go;

  /** Let a link through: its sheet opens, or — the listing's, with an
   *  account on its host — it goes straight on, which is `true`. */
  const admit = (link: AppLinkArrival, id: number): boolean => {
    const round = ++sequence.current;
    if (link.kind === "listing") {
      const verdict = listingVerdict(link, contextRef.current().cloudHosts);
      if (verdict.kind === "direct") {
        setHeld(null);
        goRef.current(verdict.destination);
        return true;
      }
      setAnswered({ round, verdict });
    }
    setHeld({ link, phase: "open", awaitsAccount: false, id, round });
    return false;
  };
  const admitRef = useRef(admit);
  admitRef.current = admit;

  useEffect(() => {
    if (!ready) return;
    let live = true;
    const take = () => {
      void projectSyncApi.takeAppLink().then((link) => {
        if (!live || !link) return;
        const id = ++sequence.current;
        // A link that arrives over another's sheet takes the sheet's place.
        if (heldRef.current?.phase !== "open" && waitsRef.current()) {
          setHeld({ link, phase: "waiting", awaitsAccount: false, id, round: id });
          return;
        }
        admitRef.current(link, id);
      }).catch(() => undefined);
    };
    // One that started the app is already waiting; the rest are announced.
    take();
    const listening = projectSyncApi.onAppLink(take);
    listening.catch(() => undefined);
    return () => {
      live = false;
      void listening.then((unlisten) => unlisten()).catch(() => undefined);
    };
  }, [ready]);

  const open = held?.phase === "open" && (held.link.kind === "repository" || held.link.kind === "session")
    ? { round: held.round, host: held.link.host, address: held.link.address, session: held.link.kind === "session" ? held.link.resource_id : null }
    : null;
  useEffect(() => {
    if (!open) return;
    let live = true;
    const { host, address, round, session } = open;
    void (async () => {
      let verdict: LinkVerdict;
      try {
        const answer = await projectSyncApi.resolveAppLink(host, address);
        const places: Record<string, CatalogCloudPlace | null> = {};
        for (const seer of catalogSeers(answer)) {
          places[seer.storageId] = await storageCatalogApi.findCloudRepository(seer.storageId, seer.repositoryId);
        }
        verdict = linkVerdict(answer, { ...contextRef.current(), places }, session);
      } catch (reason) {
        verdict = unanswered(errorMessage(reason));
      }
      if (live) setAnswered({ round, verdict });
    })();
    return () => {
      live = false;
    };
  }, [open?.round]);

  /* An answer belongs to the asking it came from: until this round's is in,
     the sheet is looking. A link that could not be read has nothing to ask. */
  const sheet: AppLinkSheetState = held?.link.kind === "invalid"
    ? { phase: "answered", verdict: INVALID_LINK }
    : held && answered?.round === held.round ? { phase: "answered", verdict: answered.verdict } : { phase: "looking" };

  return {
    held,
    sheet,
    /** Open on the waiting line. */
    open: () => {
      const current = heldRef.current;
      if (current?.phase === "waiting") admit(current.link, current.id);
    },
    /** Ask the service again. */
    retry: () => {
      const round = ++sequence.current;
      setHeld((current) => (current ? { ...current, round } : current));
    },
    /** Mallard Cloud is being set up: the link waits for the account. */
    setAside: () => setHeld((current) => (current ? { ...current, phase: "waiting", awaitsAccount: true } : current)),
    /** Whether a link is set aside for a Mallard Cloud account. */
    awaitsAccount: () => heldRef.current?.awaitsAccount ?? false,
    /** A Mallard Cloud account was added: the link set aside for one is let
     *  through. `true` when it went straight to its page. */
    resume: (): boolean => {
      const current = heldRef.current;
      return current?.awaitsAccount ? admit(current.link, current.id) : false;
    },
    /** Let the link go. With an `id`, only that arrival: a sheet that closes
     *  late must not take a newer link with it. */
    drop: (id?: number) => setHeld((current) => (current && (id === undefined || current.id === id) ? null : current)),
  };
}
