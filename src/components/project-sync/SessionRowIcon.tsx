// Lucide 0.468.0 (ISC) and Tabler Icons v3.46.0 (MIT), sized for session rows.
// Sources and license: THIRD_PARTY_NOTICES.md.
const paths = {
  "message": ["M21 15a2 2 0 0 1-2 2H7l-4 4V5a2 2 0 0 1 2-2h14a2 2 0 0 1 2 2z"],
  "tool": ["M14.7 6.3a1 1 0 0 0 0 1.4l1.6 1.6a1 1 0 0 0 1.4 0l3.77-3.77a6 6 0 0 1-7.94 7.94l-6.91 6.91a2.12 2.12 0 0 1-3-3l6.91-6.91a6 6 0 0 1 7.94-7.94l-3.76 3.76z"],
  "token": ["M4 18v-5l4 -2l4 2v5l-4 2l-4 -2", "M8 11v-5l4 -2l4 2v5", "M12 13l4 -2l4 2v5l-4 2l-4 -2"],
  "clock": ["M3 12a9 9 0 1 0 18 0a9 9 0 0 0 -18 0", "M12 7v5l3 3"],
  "info": ["M3 5a2 2 0 0 1 2 -2h14a2 2 0 0 1 2 2v14a2 2 0 0 1 -2 2h-14a2 2 0 0 1 -2 -2v-14", "M12 8h.01", "M12 16v-4"],
  "chevron-right": ["M9 6l6 6l-6 6"],
  "activity": ["M3 12h4l3 8l4 -16l3 8h4"],
} as const;

export default function SessionRowIcon({ name }: { name: keyof typeof paths }) {
  return (
    <svg
      aria-hidden="true"
      data-icon={name}
      className="session-row-icon"
      focusable="false"
      width={14}
      height={14}
      viewBox="0 0 24 24"
      fill="none"
      stroke="currentColor"
      strokeWidth={name === "info" ? 1.75 : 2.5}
      strokeLinecap="round"
      strokeLinejoin="round"
    >
      {paths[name].map((d) => <path key={d} d={d} />)}
    </svg>
  );
}
