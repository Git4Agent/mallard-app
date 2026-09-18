const paths = {
  appearance: [
    "M14 10a4 4 0 1 1-8 0 4 4 0 0 1 8 0Z",
    "M10 1v2m0 14v2M1 10h2m14 0h2M3.5 3.5 5 5m10 10 1.5 1.5m0-13L15 5M5 15l-1.5 1.5",
  ],
  privacy: [
    "M6 8V6a4 4 0 0 1 8 0v2",
    "M5 8h10a1 1 0 0 1 1 1v8a1 1 0 0 1-1 1H5a1 1 0 0 1-1-1V9a1 1 0 0 1 1-1Z",
    "M10 12v2",
  ],
  sync: ["M3 6h14m-3-3 3 3-3 3M17 14H3m3-3-3 3 3 3"],
  performance: ["M11.5 2 4 11h5l-.5 7L16 9h-5l.5-7Z"],
  logs: [
    "M5 2h10a1 1 0 0 1 1 1v14a1 1 0 0 1-1 1H5a1 1 0 0 1-1-1V3a1 1 0 0 1 1-1Z",
    "M7 6h6M7 10h6M7 14h4",
  ],
  display: ["M18 10a8 8 0 1 1-16 0 8 8 0 0 1 16 0Z", "M10 5v5l3 2"],
  "local-data": [
    "M2 11l3-7h10l3 7v5a1 1 0 0 1-1 1H3a1 1 0 0 1-1-1v-5Zm0 0h16",
    "M14 14h1",
  ],
} as const;

export default function SettingsSectionIcon({ name }: { name: keyof typeof paths }) {
  return (
    <svg
      aria-hidden="true"
      focusable="false"
      className="settings-section-icon"
      width={20}
      height={20}
      viewBox="0 0 20 20"
      fill="none"
      stroke="currentColor"
      strokeWidth={1.8}
      strokeLinecap="round"
      strokeLinejoin="round"
    >
      {paths[name].map((d) => <path key={d} d={d} />)}
    </svg>
  );
}
