import { useEffect, useRef, useState } from "react";
import Icon from "../Icons";

export const WELCOME_RIDES = ["bicycle", "rocket", "submarine"] as const;
export type WelcomeRide = (typeof WELCOME_RIDES)[number];

/** 36 frames of 90 ms. One ride ends inside WCAG 2.2.2's five seconds, so
 *  the plate needs no pause control. */
export const WELCOME_RIDE_MS = 3240;

/** Any ride but the one shown last, so reopening the page shows a new one. */
export function pickWelcomeRide(previous: WelcomeRide | null, random: () => number = Math.random): WelcomeRide {
  const choices = WELCOME_RIDES.filter((ride) => ride !== previous);
  return choices[Math.floor(random() * choices.length)] ?? choices[0]!;
}

let lastRide: WelcomeRide | null = null;
/* A GIF restarts only as a new image, so every play gets its own URL. */
let plays = 0;

/**
 * The Projects landing on a machine with no projects: a mallard ride, the
 * heading, and the two ways Add starts.
 *
 * The plate rides once and rests on its first frame (the still, drawn under
 * the ride so the swap never blanks); a pointer over it rides again. With
 * reduced motion only the still is shown. Motion starts on the client, so a
 * first paint is always the still.
 */
export default function ProjectsWelcome({ onAddProject, onAddCloudProject }: {
  onAddProject: () => void;
  onAddCloudProject: () => void;
}) {
  const [ride] = useState(() => {
    lastRide = pickWelcomeRide(lastRide);
    return lastRide;
  });
  const [play, setPlay] = useState<number | null>(null);
  const restTimer = useRef<number | undefined>(undefined);

  const startRide = () => {
    if (window.matchMedia("(prefers-reduced-motion: reduce)").matches) return;
    plays += 1;
    setPlay(plays);
  };

  useEffect(() => {
    startRide();
    return () => window.clearTimeout(restTimer.current);
  }, []);

  return (
    <section className="projects-welcome" aria-labelledby="projects-welcome-title">
      <div
        className="projects-welcome-art"
        aria-hidden="true"
        onPointerEnter={() => {
          if (play === null) startRide();
        }}
      >
        <img src={`/welcome/mallard-${ride}-still.svg`} alt="" width={1200} height={560} />
        {play !== null && (
          <img
            src={`/welcome/mallard-${ride}-ride.gif?play=${play}`}
            alt=""
            width={1200}
            height={560}
            onLoad={() => {
              window.clearTimeout(restTimer.current);
              restTimer.current = window.setTimeout(() => setPlay(null), WELCOME_RIDE_MS);
            }}
          />
        )}
      </div>
      <h1 id="projects-welcome-title">Add your first project</h1>
      <div className="projects-welcome-actions">
        <button type="button" className="btn btn-primary" onClick={onAddProject}>
          <Icon name="plus" size={13} />
          Add project…
        </button>
        <button type="button" className="btn btn-ghost" onClick={onAddCloudProject}>
          <Icon name="cloud" size={14} />
          From Mallard Cloud…
        </button>
      </div>
    </section>
  );
}
