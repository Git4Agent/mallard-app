import type { SVGProps } from "react";

// Pixel geometry uses 24 px assets from HackerNoon's Pixel Icon Library and
// Pixelarticons. The ChatGPT mark is a crisp-edge adaptation of its canonical
// knot geometry. Semantic gaps follow the same construction; see
// THIRD_PARTY_NOTICES.md.
export type IconName =
  | "activity"
  | "alert-triangle"
  | "ban"
  | "catalog"
  | "check"
  | "chevron-down"
  | "chevron-left"
  | "chevron-right"
  | "chevron-up"
  | "claude"
  | "clock"
  | "cloud"
  | "code"
  | "computer"
  | "copy"
  | "check-circle"
  | "download"
  | "drive"
  | "edit"
  | "file"
  | "folder"
  | "external-link"
  | "git-branch"
  | "git-commit"
  | "git-compare-arrows"
  | "git-fork"
  | "git-folder"
  | "info"
  | "lightbulb"
  | "link"
  | "link-off"
  | "lock"
  | "message"
  | "moon"
  | "more"
  | "octagon-x"
  | "openai"
  | "paw"
  | "plus"
  | "puzzle"
  | "range"
  | "refresh"
  | "settings"
  | "sidebar-collapse"
  | "sidebar-expand"
  | "sun"
  | "token"
  | "tool"
  | "trash"
  | "terminal"
  | "upload"
  | "user"
  | "x";

interface IconProps extends SVGProps<SVGSVGElement> {
  name: IconName;
  size?: number;
}

/**
 * Every mark below is drawn as whole cells on a 24-unit grid, and the SVG asks
 * for crispEdges — so a cell is never antialiased, it is *rounded*. A stem
 * therefore keeps one uniform width only when `size × devicePixelRatio ÷ 24` is
 * a whole number. On the Retina displays this app targets that means **12 or
 * 24**; at 13, 14, 17 or 18 the same shape comes out with some stems one device
 * pixel and others two, which is what reads as "not pixel art".
 *
 * 12 for inline and button icons, 24 for the large glyph on an empty or error
 * state. The `size = 14` default predates this and is still what most of the
 * app passes; new code should name 12 or 24 explicitly.
 */
export default function Icon({ name, size = 14, ...props }: IconProps) {
  return (
    <svg
      aria-hidden="true"
      data-icon={name}
      fill="none"
      height={size}
      shapeRendering="crispEdges"
      viewBox="0 0 24 24"
      width={size}
      {...props}
    >
      {name === "activity" && (
        <path
          className="icon-pixel-fill icon-pixelarticons"
          fill="currentColor"
          stroke="none"
          d="M4 20h18v2H4ZM2 2h2v18H2ZM7 6h2v12H7ZM12 10h2v8h-2ZM17 14h2v4h-2Z"
        />
      )}
      {name === "alert-triangle" && (
        <g className="icon-pixel-fill" fill="currentColor" stroke="none">
          <path d="M14 11v3h-1v3h-2v-3h-1v-3h4Z" />
          <path d="M11 18h2v2h-2Z" />
          <path d="M22 20v-2h-1v-2h-1v-2h-1v-2h-1v-2h-1V8h-1V6h-1V4h-1V2h-1V1h-2v1h-1v2H9v2H8v2H7v2H6v2H5v2H4v2H3v2H2v2H1v2h1v1h20v-1h1v-2h-1ZM3 21v-1h1v-2h1v-2h1v-2h1v-2h1v-2h1V8h1V6h1V4h2v2h1v2h1v2h1v2h1v2h1v2h1v2h1v2h1v1H3Z" />
        </g>
      )}
      {name === "ban" && (
        <g className="icon-pixel-fill" fill="currentColor" stroke="none">
          <path d="M9 1h6v1h2v1h2v1h1v1h1v2h1v2h1v6h-1v2h-1v2h-1v1h-1v1h-2v1h-2v1H9v-1H7v-1H5v-1H4v-1H3v-2H2v-2H1V9h1V7h1V5h1V4h1V3h2V2h2V1Zm0 2v1H7v1H6l13 13v-1h1v-2h1V9h-1V7h-1V6h-1V5h-1V4h-2V3H9ZM5 6v1H4v2H3v6h1v2h1v1h1v1h1v1h2v1h6v-1h2v-1h1L5 6Z" />
        </g>
      )}
      {name === "catalog" && (
        <path
          className="icon-pixel-fill"
          fill="currentColor"
          stroke="none"
          d="M4 4h5v5H4ZM15 4h5v5h-5ZM4 15h5v5H4ZM15 15h5v5h-5Z"
        />
      )}
      {name === "cloud" && (
        <path className="icon-pixel-fill" fill="currentColor" stroke="none" d="M22 12v-1h-2V9h-1V8h-1V7h-3v1h-1V6h-1V5H8v1H7v1H6v1H5v3H2v1H1v5h1v1h1v1h2v1h14v-1h2v-1h1v-1h1v-5h-1Zm0 4h-1v1h-1v1h-2v1H6v-1H4v-1H3v-1H2v-3h1v-1h3V9h1V8h1V7h1V6h3v1h1v2h3V8h1v1h1v1h1v2h2v1h1v3Z" />
      )}
      {/* The two chevrons need a gap wide enough to survive 12px: at a 2-unit
          separation their arms nearly met and the pair read as a ring of dots
          rather than as < >. Four units apart they stay two marks. */}
      {name === "code" && (
        <g className="icon-pixel-fill" fill="currentColor" stroke="none">
          <path d="M8 6h2v2H8ZM6 8h2v2H6ZM4 10h2v4H4ZM6 14h2v2H6ZM8 16h2v2H8Z" />
          <path d="M14 6h2v2h-2ZM16 8h2v2h-2ZM18 10h2v4h-2ZM16 14h2v2h-2ZM14 16h2v2h-2Z" />
        </g>
      )}
      {name === "copy" && (
        <g className="icon-pixel-fill" fill="currentColor" stroke="none">
          <path d="M8 8h13v13H8Zm1 1v11h11V9Z" />
          <path d="M3 3h13v5h-1V4H4v11h4v1H3Z" />
        </g>
      )}
      {name === "computer" && (
        <g className="icon-pixel-fill" fill="currentColor" stroke="none">
          <path d="M19 16h1v1h-1Z" />
          <path d="M22 4V3h-5v1h-1v16h1v1h5v-1h1V4h-1Zm-1 10v5h-3v-5h3Zm-3-2v-2h3v2h-3Zm0-4V6h3v2h-3Z" />
          <path d="M4 19h10v2H4Z" />
          <path d="M3 5v10h11v2H2v-1H1V4h1V3h12v2H3Z" />
        </g>
      )}
      {/* A menu check mark without an enclosing ring at its rendered size. */}
      {name === "check" && (
        <path
          className="icon-pixel-fill"
          fill="currentColor"
          stroke="none"
          d="M19 9v1h-1v1h-1v1h-1v1h-1v1h-1v1h-1v1h-1v1h-2v-1H9v-1H8v-1H7v-1H6v-1h1v-1h1v1h1v1h1v1h2v-1h1v-1h1v-1h1v-1h1V9h1V8h1v1h1Z"
        />
      )}
      {name === "check-circle" && (
        <g className="icon-pixel-fill" fill="currentColor" stroke="none">
          <path d="M19 9v1h-1v1h-1v1h-1v1h-1v1h-1v1h-1v1h-1v1h-2v-1H9v-1H8v-1H7v-1H6v-1h1v-1h1v1h1v1h1v1h2v-1h1v-1h1v-1h1v-1h1V9h1V8h1v1h1Z" />
          <path d="M22 9V7h-1V5h-1V4h-1V3h-2V2h-2V1H9v1H7v1H5v1H4v1H3v2H2v2H1v6h1v2h1v2h1v1h1v1h2v1h2v1h6v-1h2v-1h2v-1h1v-1h1v-2h1v-2h1V9h-1Zm-2 6v2h-1v2h-2v1h-2v1H9v-1H7v-1H5v-2H4v-2H3V9h1V7h1V5h2V4h2V3h6v1h2v1h2v2h1v2h1v6h-1Z" />
        </g>
      )}
      {name === "info" && (
        <g className="icon-pixel-fill icon-pixelarticons" fill="currentColor" stroke="none">
          <path d="M6 2h12v2H6ZM6 20h12v2H6ZM2 6h2v12H2ZM20 6h2v12h-2ZM18 4h2v2h-2ZM18 18h2v2h-2ZM4 4h2v2H4ZM4 18h2v2H4Z" />
          <path d="M11 7h2v2h-2ZM11 11h2v6h-2Z" />
        </g>
      )}
      {name === "chevron-down" && <path className="icon-pixel-fill" fill="currentColor" stroke="none" d="M20 8v2h-1v1h-1v1h-1v1h-1v1h-1v1h-1v1h-4v-1H9v-1H8v-1H7v-1H6v-1H5v-1H4V8h2v1h1v1h1v1h1v1h1v1h4v-1h1v-1h1v-1h1V9h1V8h2Z" />}
      {name === "chevron-left" && <path className="icon-pixel-fill" fill="currentColor" stroke="none" d="M16 4v2h-1v1h-1v1h-1v1h-1v1h-1v4h1v1h1v1h1v1h1v1h1v2h-2v-1h-1v-1h-1v-1h-1v-1h-1v-1H9v-1H8v-4h1V9h1V8h1V7h1V6h1V5h1V4h2Z" />}
      {name === "chevron-right" && <path className="icon-pixel-fill" fill="currentColor" stroke="none" d="M8 4h2v1h1v1h1v1h1v1h1v1h1v1h1v4h-1v1h-1v1h-1v1h-1v1h-1v1h-1v1H8v-2h1v-1h1v-1h1v-1h1v-1h1v-4h-1V9h-1V8h-1V7H9V6H8V4Z" />}
      {name === "chevron-up" && <path className="icon-pixel-fill" fill="currentColor" stroke="none" d="M10 8h4v1h1v1h1v1h1v1h1v1h1v1h1v2h-2v-1h-1v-1h-1v-1h-1v-1h-1v-1h-4v1H9v1H8v1H7v1H6v1H4v-2h1v-1h1v-1h1v-1h1v-1h1V9h1V8Z" />}
      {/* PLACEHOLDER for the Claude Code mark: an eight-ray tapered burst, rays
          converging at the centre and widening to the tip. The taper is what
          separates it from a generic asterisk and the part that still reads at
          14px. Vector rather than pixel art, and geometricPrecision rather than
          the sheet's crispEdges — the same call the ChatGPT knot makes, because
          unantialiased diagonals shatter. Needs the canonical geometry and a
          THIRD_PARTY_NOTICES.md entry before it can claim to be the real mark. */}
      {name === "claude" && (
        <path
          className="icon-claude-mark"
          fill="currentColor"
          stroke="none"
          shapeRendering="geometricPrecision"
          d="M12 12L10.1 1.5L13.9 1.5ZM12 12L18.08 3.23L20.77 5.92ZM12 12L22.5 10.1L22.5 13.9ZM12 12L20.77 18.08L18.08 20.77ZM12 12L13.9 22.5L10.1 22.5ZM12 12L5.92 20.77L3.23 18.08ZM12 12L1.5 13.9L1.5 10.1ZM12 12L3.23 5.92L5.92 3.23Z"
        />
      )}
      {name === "clock" && (
        <g className="icon-pixel-fill" fill="currentColor" stroke="none">
          <path d="M22 9V7h-1V5h-1V4h-1V3h-2V2h-2V1H9v1H7v1H5v1H4v1H3v2H2v2H1v6h1v2h1v2h1v1h1v1h2v1h2v1h6v-1h2v-1h2v-1h1v-1h1v-2h1v-2h1V9h-1Zm-1 6h-1v2h-1v2h-2v1h-2v1H9v-1H7v-1H5v-2H4v-2H3V9h1V7h1V5h2V4h2V3h6v1h2v1h2v2h1v2h1v6Z" />
          <path d="M16 15v1h-1v1h-1v-1h-1v-1h-1v-1h-1V5h2v8h1v1h1v1h1Z" />
        </g>
      )}
      {name === "lightbulb" && (
        /* Skills. The Library's three kind glyphs are the only marks here drawn
           to a measured brief rather than by eye: the shipped row runs 15-48
           filled cells of 144 and every mark but the cog is a one-cell outline,
           so these are outlines too — a solid fill lands at 52-81 and reads as
           a blot beside its neighbours.

           A bulb because it is the only glyph any shipped developer tool binds
           to a skill: the Material Icon Theme maps `skill.md` and a `skills/`
           folder to one. No icon system has a mark for a loadable capability —
           Material, SF Symbols, Codicons, Octicons and Fluent all return
           nothing for "skill". Nothing else in the set is a dome over a stem. */
        <path
          className="icon-pixel-fill icon-mallard-pixel"
          fill="currentColor"
          stroke="none"
          d="M8 2h8v2H8ZM6 4h2v2H6ZM16 4h2v2H16ZM4 6h2v6H4ZM18 6h2v6H18ZM6 12h2v2H6ZM16 12h2v2H16ZM8 14h8v4H8ZM10 18h4v2H10Z"
        />
      )}
      {name === "link" && (
        <g className="icon-pixel-fill" fill="currentColor" stroke="none">
          <path d="M16 10h1v7h-1v1h-1v1h-1v1h-1v1h-1v1h-1v1H5v-1H4v-1H3v-1H2v-1H1v-5h1v-1h1v-1h1v-1h1v3H4v1H3v3h1v1h1v1h1v1h4v-1h1v-1h1v-1h1v-1h1v-1h1v-5h-1v-1h-1V9h1V8h1v1h1v1Z" />
          <path d="M23 5v5h-1v1h-1v1h-1v1h-1v-3h1V9h1V6h-1V5h-1V4h-1V3h-4v1h-1v1h-1v1h-1v1h-1v1H9v5h1v1h1v1h-1v1H9v-1H8v-1H7V7h1V6h1V5h1V4h1V3h1V2h1V1h6v1h1v1h1v1h1v1h1Z" />
        </g>
      )}
      {/* The storage stays; the full-span diagonal cancels the relation. */}
      {name === "link-off" && (
        <g className="icon-pixel-fill" fill="currentColor" stroke="none">
          <path d="M16 10h1v7h-1v1h-1v1h-1v1h-1v1h-1v1h-1v1H5v-1H4v-1H3v-1H2v-1H1v-5h1v-1h1v-1h1v-1h1v3H4v1H3v3h1v1h1v1h1v1h4v-1h1v-1h1v-1h1v-1h1v-1h1v-5h-1v-1h-1V9h1V8h1v1h1v1Z" />
          <path d="M23 5v5h-1v1h-1v1h-1v1h-1v-3h1V9h1V6h-1V5h-1V4h-1V3h-4v1h-1v1h-1v1h-1v1h-1v1H9v5h1v1h1v1h-1v1H9v-1H8v-1H7V7h1V6h1V5h1V4h1V3h1V2h1V1h6v1h1v1h1v1h1v1h1Z" />
          <path d="M3 1h2v1h1v1h1v1h1v1h1v1h1v1h1v1h1v1h1v1h1v1h1v1h1v1h1v1h1v1h1v1h1v1h1v1h1v2h-2v-1h-1v-1h-1v-1h-1v-1h-1v-1h-1v-1h-1v-1h-1v-1h-1v-1h-1v-1h-1v-1H9V9H8V8H7V7H6V6H5V5H4V4H3V3H2V2h1V1Z" />
        </g>
      )}
      {name === "lock" && (
        <path
          className="icon-pixel-fill"
          fill="currentColor"
          stroke="none"
          d="M21 12v-1h-3V5h-1V3h-1V2h-2V1h-4v1H8v1H7v2H6v6H3v1H2v10h1v1h18v-1h1V12h-1Zm-1 1v8H4v-8h16ZM9 5V4h1V3h4v1h1v1h1v6H8V5h1Z"
        />
      )}
      {name === "message" && (
        <path
          className="icon-pixel-fill icon-pixelarticons"
          fill="currentColor"
          stroke="none"
          d="M20 2H4v2h16ZM20 16H6v2h14ZM22 4h-2v12h2ZM4 4H2v18h2ZM6 18H4v2h2Z"
        />
      )}
      {name === "moon" && (
        <path
          className="icon-pixel-fill icon-pixelarticons"
          fill="currentColor"
          stroke="none"
          d="M6 2h8v2h-2v2h-2V4H6V2ZM4 6V4h2v2H4Zm0 10H2V6h2v10Zm2 2H4v-2h2v2Zm2 2H6v-2h2v2Zm10 0v2H8v-2h10Zm2-2v2h-2v-2h2Zm-2-4h2v4h2v-8h-2v2h-2v2Zm-6 0v2h6v-2h-6Zm-2-2h2v2h-2v-2Zm0 0V6H8v6h2Z"
        />
      )}
      {name === "download" && (
        <path
          className="icon-pixel-fill icon-pixelarticons"
          fill="currentColor"
          stroke="none"
          d="M4 20h16v2H4ZM11 2h2v16h-2ZM13 14h2v2h-2ZM15 12h2v2h-2ZM17 10h2v2h-2ZM9 14h2v2H9ZM7 12h2v2H7ZM5 10h2v2H5Z"
        />
      )}
      {name === "drive" && (
        <path
          className="icon-pixel-fill icon-pixelarticons"
          fill="currentColor"
          stroke="none"
          d="M3 2h18v2H3ZM3 7h18v2H3ZM1 4h2v3H1ZM21 4h2v3h-2ZM19 9h2v11h-2ZM3 9h2v11H3ZM5 20h14v2H5ZM9 11h6v2H9Z"
        />
      )}
      {name === "edit" && (
        <g className="icon-pixel-fill" fill="currentColor" stroke="none">
          <path d="M22 4v3h-1v1h-1V7h-1V6h2V5h-1V4h-1v2h-1V5h-1V4h1V3h3v1h1Z" />
          <path d="M18 14v7h-1v1H2v-1H1V6h1V5h12v1h-1v1H3v13h13v-5h1v-1h1Z" />
          <path d="M18 8V7h-1V6h-2v1h-1v1h-1v1h-1v1h-1v1h-1v1H9v1H8v1H7v4h4v-1h1v-1h1v-1h1v-1h1v-1h1v-1h1v-1h1v-1h1V9h-1Zm-1 2h-1v1h-1v1h-1v1h-1v1h-1v1h-1v1h-1v1H8v-2h1v-1h1v-1h1v-1h1v-1h1v-1h1V9h1V8h2v2Z" />
        </g>
      )}
      {name === "file" && (
        <path
          className="icon-pixel-fill"
          fill="currentColor"
          stroke="none"
          d="M18 1v1h1v1h1v1h1v1h1v1h1v16h-1v1H5v-1H4V2h1V1h13Zm-1 2H6v18h15V8h-4V3Zm2 1v2h2V5h-1V4h-1Z"
        />
      )}
      {name === "folder" && (
        <path
          className="icon-pixel-fill icon-pixelarticons"
          fill="currentColor"
          stroke="none"
          d="M4 4h6v2H4ZM4 18h16v2H4ZM20 8h2v10h-2ZM2 6h2v12H2ZM10 6h10v2H10Z"
        />
      )}
      {/* Redrawn in-house on the even grid, so it carries no provenance class.
          The sourced glyph stepped its diagonal one unit at a time; at the 12px
          this renders at, a 24-unit grid is half a device pixel per unit, so the
          shaft and the frame both dissolved. Every edge here lands on an even
          coordinate: the box loses its top-right corner and the arrow leaves
          through the gap. */}
      {name === "external-link" && (
        <path className="icon-pixel-fill" fill="currentColor" stroke="none"
          d="M2 6h8v2H2ZM2 6h2v16H2ZM2 20h18v2H2ZM18 12h2v10h-2ZM12 0h10v2H12ZM20 0h2v10h-2ZM18 2h2v2h-2ZM16 4h2v2h-2ZM14 6h2v2h-2ZM12 8h2v2h-2ZM10 10h2v2h-2Z" />
      )}
      {name === "git-branch" && (
        <path className="icon-git-branch-mark icon-pixel-fill" fill="currentColor" stroke="none" d="M20 2V1h-4v1h-1v4h1v1h1v4H7V7h1V6h1V2H8V1H4v1H3v4h1v1h1v10H4v1H3v4h1v1h4v-1h1v-4H8v-1H7v-4h12V7h1V6h1V2h-1ZM5 3h2v2H5Zm2 18H5v-2h2ZM19 5h-2V3h2Z" />
      )}
      {name === "git-commit" && (
        <g className="icon-pixel-fill" fill="currentColor" stroke="none">
          <path d="M1 11h6v2H1Z" />
          <path d="M17 11h6v2h-6Z" />
          <path d="M9 7h6v2H9Z" />
          <path d="M9 15h6v2H9Z" />
          <path d="M7 9h2v6H7Z" />
          <path d="M15 9h2v6h-2Z" />
        </g>
      )}
      {name === "git-compare-arrows" && (
        <g className="icon-pixel-fill" fill="currentColor" stroke="none">
          <path d="M8 15h1v2H8v1H7v1H1v-2h6v-1h1v-1Z" />
          <path d="M21 16h1v2h-1v1h-1v1h-1v1h-1v-3h-4v-1h-1v-1h-1v-2h-1v-1h-1v-2H9v-1H8V8H7V7H1V5h7v1h1v2h1v2h1v1h1v2h1v1h1v2h4v-3h1v1h1v1h1v1Z" />
          <path d="M22 5v2h-1v1h-1v1h-1v1h-1V7h-4v1h-1v1h-1V7h1V6h1V5h4V2h1v1h1v1h1v1h1Z" />
        </g>
      )}
      {name === "git-fork" && (
        <path
          className="icon-pixel-fill icon-pixelarticons"
          fill="currentColor"
          stroke="none"
          d="M4 14h4v2H4ZM4 20h4v2H4ZM2 16h2v4H2ZM8 16h2v4H8ZM16 2h4v2h-4ZM16 8h4v2h-4ZM14 4h2v4h-2ZM20 4h2v4h-2ZM12 17h5v2h-5ZM17 12h2v5h-2ZM5 2h2v10H5Z"
        />
      )}
      {name === "git-folder" && (
        <>
          <path
            className="icon-git-folder-shell icon-pixel-fill icon-pixelarticons"
            fill="currentColor"
            stroke="none"
            d="M4 4h6v2H4ZM4 18h16v2H4ZM20 8h2v10h-2ZM2 6h2v12H2ZM10 6h10v2H10Z"
          />
          <g className="icon-git-folder-mark icon-pixel-fill" fill="currentColor" stroke="none">
            <path d="M7 10h2v2H7ZM9 12h2v2H9ZM7 14h2v2H7ZM12 14h5v2h-5Z" />
          </g>
        </>
      )}
      {name === "more" && (
        <path
          className="icon-pixel-fill icon-pixelarticons"
          fill="currentColor"
          stroke="none"
          d="M3 9h2v2H3ZM11 9h2v2h-2ZM19 9h2v2h-2ZM1 11h2v2H1ZM9 11h2v2H9ZM17 11h2v2h-2ZM3 13h2v2H3ZM11 13h2v2h-2ZM19 13h2v2h-2ZM5 11h2v2H5ZM13 11h2v2h-2ZM21 11h2v2h-2Z"
        />
      )}
      {name === "octagon-x" && (
        <g className="icon-pixel-fill" fill="currentColor" stroke="none">
          <path d="M17 8v2h-1v1h-1v2h1v1h1v2h-1v1h-2v-1h-1v-1h-2v1h-1v1H8v-1H7v-2h1v-1h1v-2H8v-1H7V8h1V7h2v1h1v1h2V8h1V7h2v1h1Z" />
          <path d="M22 8V7h-1V6h-1V5h-1V4h-1V3h-1V2h-1V1H8v1H7v1H6v1H5v1H4v1H3v1H2v1H1v8h1v1h1v1h1v1h1v1h1v1h1v1h1v1h8v-1h1v-1h1v-1h1v-1h1v-1h1v-1h1v-1h1V8h-1Zm-2 7v1h-1v1h-1v1h-1v1h-1v1h-1v1H9v-1H8v-1H7v-1H6v-1H5v-1H4v-1H3V9h1V8h1V7h1V6h1V5h1V4h1V3h6v1h1v1h1v1h1v1h1v1h1v1h1v6h-1Z" />
        </g>
      )}
      {name === "openai" && (
        <path
          className="icon-pixel-fill icon-chatgpt-mark"
          fill="currentColor"
          stroke="none"
          transform="translate(1 1) scale(0.06875)"
          d="m297.06 130.97c7.26-21.79 4.76-45.66-6.85-65.48-17.46-30.4-52.56-46.04-86.84-38.68-15.25-17.18-37.16-26.95-60.13-26.81-35.04-.08-66.13 22.48-76.91 55.82-22.51 4.61-41.94 18.7-53.31 38.67-17.59 30.32-13.58 68.54 9.92 94.54-7.26 21.79-4.76 45.66 6.85 65.48 17.46 30.4 52.56 46.04 86.84 38.68 15.24 17.18 37.16 26.95 60.13 26.8 35.06.09 66.16-22.49 76.94-55.86 22.51-4.61 41.94-18.7 53.31-38.67 17.57-30.32 13.55-68.51-9.94-94.51zm-120.28 168.11c-14.03.02-27.62-4.89-38.39-13.88.49-.26 1.34-.73 1.89-1.07l63.72-36.8c3.26-1.85 5.26-5.32 5.24-9.07v-89.83l26.93 15.55c.29.14.48.42.52.74v74.39c-.04 33.08-26.83 59.9-59.91 59.97zm-128.84-55.03c-7.03-12.14-9.56-26.37-7.15-40.18.47.28 1.3.79 1.89 1.13l63.72 36.8c3.23 1.89 7.23 1.89 10.47 0l77.79-44.92v31.1c.02.32-.13.63-.38.83l-64.41 37.19c-28.69 16.52-65.33 6.7-81.92-21.95zm-16.77-139.09c7-12.16 18.05-21.46 31.21-26.29 0 .55-.03 1.52-.03 2.2v73.61c-.02 3.74 1.98 7.21 5.23 9.06l77.79 44.91-26.93 15.55c-.27.18-.61.21-.91.08l-64.42-37.22c-28.63-16.58-38.45-53.21-21.95-81.89zm221.26 51.49-77.79-44.92 26.93-15.54c.27-.18.61-.21.91-.08l64.42 37.19c28.68 16.57 38.51 53.26 21.94 81.94-7.01 12.14-18.05 21.44-31.2 26.28v-75.81c.03-3.74-1.96-7.2-5.2-9.06zm26.8-40.34c-.47-.29-1.3-.79-1.89-1.13l-63.72-36.8c-3.23-1.89-7.23-1.89-10.47 0l-77.79 44.92v-31.1c-.02-.32.13-.63.38-.83l64.41-37.16c28.69-16.55 65.37-6.7 81.91 22 6.99 12.12 9.52 26.31 7.15 40.1zm-168.51 55.43-26.94-15.55c-.29-.14-.48-.42-.52-.74v-74.39c.02-33.12 26.89-59.96 60.01-59.94 14.01 0 27.57 4.92 38.34 13.88-.49.26-1.33.73-1.89 1.07l-63.72 36.8c-3.26 1.85-5.26 5.31-5.24 9.06l-.04 89.79zm14.63-31.54 34.65-20.01 34.65 20v40.01l-34.65 20-34.65-20z"
        />
      )}
      {name === "paw" && (
        /* Pets, both formats. Four toes on an arc over a pad — `pets` is
           literally what Material calls this glyph, and `pawprint` is one of
           exactly two symbols Apple tags "pet friendly".

           The outer pair sits two rows below the inner pair, not one. A paw is
           five separate elements with four gaps, which is the most a twelve-cell
           grid will hold, and at a one-row offset the outer toes meet the inner
           ones at the corner and fuse. Two rows is also Material's own
           proportion — its `pets` drops the outer toes 160 of 960 units. No
           pixel-art set ships a paw for exactly this reason, so it is drawn
           natively here rather than shrunk from a 24-grid original. */
        <path
          className="icon-pixel-fill icon-mallard-pixel"
          fill="currentColor"
          stroke="none"
          d="M6 2h4v4H6ZM14 2h4v4H14ZM2 8h4v4H2ZM18 8h4v4H18ZM8 12h8v8H8ZM4 14h4v4H4ZM16 14h4v4H16ZM6 18h2v2H6ZM16 18h2v2H16Z"
        />
      )}
      {name === "plus" && (
        <path
          className="icon-pixel-fill icon-pixelarticons"
          fill="currentColor"
          stroke="none"
          d="M13 11h7v2h-7v7h-2v-7H4v-2h7V4h2v7Z"
        />
      )}
      {name === "puzzle" && (
        /* Plugins. The jigsaw piece is the one settled mark of the three —
           Chrome's extensions button, Material `extension` (the only icon
           Google tags `plugin`), and Fluent's `extension`, which is an alias of
           its puzzle piece.

           This is Apple's reduced cut, not the four-tabbed toy: one socket
           bitten out of the left edge, one knob on the right, top and bottom
           straight. Apple ships exactly that as `puzzlepiece.extension` in the
           Extensions pane because the four-tabbed form collapses into a blob at
           11pt. Even reduced it is the tightest fit in the set — a knob is a
           fifth of the icon's width, which is two cells here — so it is drawn
           hollow, where the socket and the knob still read as departures from a
           straight edge. */
        <path
          className="icon-pixel-fill icon-mallard-pixel"
          fill="currentColor"
          stroke="none"
          d="M2 2h16v2H2ZM2 4h2v6H2ZM16 4h2v6H16ZM4 8h4v2H4ZM18 8h4v2H18ZM6 10h2v6H6ZM20 10h2v6H20ZM2 14h4v2H2ZM16 14h4v2H16ZM2 16h2v6H2ZM16 16h2v6H16ZM4 20h12v2H4Z"
        />
      )}
      {name === "range" && (
        /* An empty span: the rail's window held open with nothing in it. The
           state it marks is not "no commits" — that is the sibling rail, which
           wears `git-commit` — but "none in *this* window", the only one of the
           three empty rails that paging could ever change. So the mark has to
           say span, not absence.

           `clock` said time instead, and at 37 ink it sat near the top of the
           set's 15-48 band, heavier than the row it introduces.

           Absence drawn as less ink does not survive here: a gapped rail, a
           dotted rail and a dashed-out commit node measured 7, 12 and 8 against
           a floor of 15, and all three dissolve at 14px. Facing brackets carry
           22 across two blobs, and no other mark in the set uses the topology.
           They also echo the bracket glyphs of the mono face this row already
           sets its dates in, beside the very range the mark stands for. */
        <g className="icon-pixel-fill icon-mallard-pixel" fill="currentColor" stroke="none">
          <path d="M5 5h6v2H7v10h4v2H5Z" />
          <path d="M19 5h-6v2h4v10h-4v2h6Z" />
        </g>
      )}
      {name === "refresh" && (
        <g className="icon-pixel-fill" fill="currentColor" stroke="none">
          <path d="M23 14v1h-1v2h-1v2h-1v1h-1v1h-2v1h-2v1H9v-1H7v-1H5v-1H3v1H2v1H1v-7h7v1H7v1H6v2h1v1h2v1h6v-1h2v-1h2v-2h1v-3h3Z" />
          <path d="M23 2v7h-7V8h1V7h1V5h-1V4h-2V3H9v1H7v1H5v2H4v3H1V9h1V7h1V5h1V4h1V3h2V2h2V1h6v1h2v1h2v1h2V3h1V2h1Z" />
        </g>
      )}
      {name === "settings" && (
        <path
          className="icon-pixel-fill"
          fill="currentColor"
          stroke="none"
          d="M15 2h4v2h3v2h-3v2h-4v-2h-13v-2h13ZM5 9h4v2h13v2h-13v2h-4v-2h-3v-2h3ZM11 16h4v2h7v2h-7v2h-4v-2h-9v-2h9Z"
        />
      )}
      {name === "sidebar-collapse" && (
        <g className="icon-pixel-fill" fill="currentColor" stroke="none">
          <path d="M22 5V3h-2V2H4v1H2v2H1v14h1v2h2v1h16v-1h2v-2h1V5h-1ZM7 19H5v-1H4V6h1V5h2v14Zm13-1h-1v1h-9V5h9v1h1v12Z" />
          <path d="M18 7v2h-1v1h-1v1h-1v2h1v1h1v1h1v2h-2v-1h-1v-1h-1v-1h-1v-1h-1v-2h1v-1h1V9h1V8h1V7h2Z" />
        </g>
      )}
      {name === "sidebar-expand" && (
        <g className="icon-pixel-fill" fill="currentColor" stroke="none">
          <path d="M22 5V3h-2V2H4v1H2v2H1v14h1v2h2v1h16v-1h2v-2h1V5h-1ZM7 19H5v-1H4V6h1V5h2v14Zm13-1h-1v1h-9V5h9v1h1v12Z" />
          <path d="M18 11v2h-1v1h-1v1h-1v1h-1v1h-2v-2h1v-1h1v-1h1v-2h-1v-1h-1V9h-1V7h2v1h1v1h1v1h1v1h1Z" />
        </g>
      )}
      {name === "sun" && (
        <path
          className="icon-pixel-fill icon-pixelarticons"
          fill="currentColor"
          stroke="none"
          d="M13 3h-2v2h2V3zm4 2h2v2h-2V5zm-6 6h2v2h-2v-2zm-8 0h2v2H3v-2zm18 0h-2v2h2v-2zM5 5h2v2H5V5zm14 14h-2v-2h2v2zm-8 2h2v-2h-2v2zm-4-2H5v-2h2v2zM9 7h6v2H9V7zm0 8H7V9h2v6zm0 0v2h6v-2h2V9h-2v6H9Z"
        />
      )}
      {name === "token" && (
        <path
          className="icon-pixel-fill icon-pixelarticons"
          fill="currentColor"
          stroke="none"
          d="M6 2h6v2H6ZM4 4h2v2H4ZM12 4h2v2h-2ZM4 12h2v2H4ZM12 12h2v2h-2ZM6 14h6v2H6ZM2 6h2v6H2ZM14 6h2v6h-2ZM14 8h4v2h-4ZM10 18h2v2h-2ZM18 10h2v2h-2ZM12 20h2v2h-2ZM18 18h2v2h-2ZM12 20h6v2h-6ZM8 14h2v4H8ZM20 12h2v6h-2ZM7 6h4v2H7ZM9 6h2v6H9ZM15 14h2v4h-2ZM14 12h3v2h-3Z"
        />
      )}
      {name === "tool" && (
        /* A cog: teeth at the cardinals, hollow hub. It matches its row
           neighbours' 20×20 optical weight and is the only
           round silhouette on the strip, so it is found by shape before it is
           read. Drawn in-house on the 12-cell coarse grid and doubled, so
           every stem is two cells wide; the sibling marks come from downloaded
           sets, hence the distinct provenance class. */
        <path
          className="icon-pixel-fill icon-mallard-pixel"
          fill="currentColor"
          stroke="none"
          d="M10 2h4v8h-4ZM6 6h4v12H6ZM14 6h4v12h-4ZM2 10h4v4H2ZM18 10h4v4h-4ZM10 14h4v8h-4Z"
        />
      )}
      {name === "trash" && (
        <path
          className="icon-pixel-fill icon-pixelarticons"
          fill="currentColor"
          stroke="none"
          d="M18 22H6v-2h12ZM9 6h6V4h2v2h5v2h-2v12h-2V8H6v12H4V8H2V6h5V4h2ZM15 4H9V2h6Z"
        />
      )}
      {/* Drawn in-house, so no provenance class. The sourced glyph framed the
          prompt in a 2-unit window and left it three 2x2 blocks in the corner;
          at the 12-14px this icon actually renders, a 24-unit grid gives half a
          device pixel per unit, so the prompt sank into a smudge and the icon
          read as an empty box. Dropping the frame buys the whole grid, and the
          chevron and cursor bar are what identify a terminal anyway. */}
      {name === "terminal" && (
        <path
          className="icon-pixel-fill"
          fill="currentColor"
          stroke="none"
          d="M4 6h2v2H4ZM6 8h2v2H6ZM8 10h2v2H8ZM10 12h2v2h-2ZM8 14h2v2H8ZM6 16h2v2H6ZM4 18h2v2H4ZM14 18h6v2h-6Z"
        />
      )}
      {name === "upload" && (
        <path
          className="icon-pixel-fill icon-pixelarticons"
          fill="currentColor"
          stroke="none"
          d="M4 4h16V2H4ZM11 22h2V6h-2ZM13 10h2V8h-2ZM15 12h2v-2h-2ZM17 14h2v-2h-2ZM9 10h2V8H9ZM7 12h2v-2H7ZM5 14h2v-2H5Z"
        />
      )}
      {name === "user" && (
        <path
          className="icon-pixel-fill icon-pixelarticons"
          fill="currentColor"
          stroke="none"
          d="M9 2h6v2H9ZM9 10h6v2H9ZM15 4h2v6h-2ZM7 4h2v6H7ZM4 18h2v4H4ZM18 18h2v4h-2ZM8 14h8v2H8ZM6 16h2v2H6ZM16 16h2v2h-2Z"
        />
      )}
      {name === "x" && (
        <path
          className="icon-pixel-fill"
          fill="currentColor"
          stroke="none"
          d="M14 13h1v1h1v1h1v1h1v1h1v1h1v1h1v1h1v1h-1v1h-1v-1h-1v-1h-1v-1h-1v-1h-1v-1h-1v-1h-1v-1h-1v-1h-1v-1h-2v1h-1v1H9v1H8v1H7v1H6v1H5v1H4v1H3v-1H2v-1h1v-1h1v-1h1v-1h1v-1h1v-1h1v-1h1v-1h1v-2H9v-1H8V9H7V8H6V7H5V6H4V5H3V4H2V3h1V2h1v1h1v1h1v1h1v1h1v1h1v1h1v1h1v1h2V9h1V8h1V7h1V6h1V5h1V4h1V3h1V2h1v1h1v1h-1v1h-1v1h-1v1h-1v1h-1v1h-1v1h-1v1h-1v2Z"
        />
      )}
    </svg>
  );
}
