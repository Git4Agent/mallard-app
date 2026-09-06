import Icon, { type IconName } from "../Icons";

/** Marks Mallard ships art for. Providers are an open registry
 *  (`types.ts`: the frontend "never holds a fixed provider array"), so anything
 *  absent here falls back to a monogram off the display name rather than to an
 *  empty slot — which is the same defect this component exists to remove. */
const PROVIDER_MARKS: Record<string, IconName> = {
  codex: "openai",
  claude: "claude-mark",
};

export default function ProviderMark({ provider, displayName, size = 14 }: {
  provider: string;
  displayName: string;
  size?: number;
}) {
  const mark = PROVIDER_MARKS[provider];
  if (mark) return <Icon name={mark} size={size} className={`sync-provider-mark ${provider}`} />;
  return (
    <span className="sync-provider-mark sync-provider-monogram" aria-hidden="true">
      {displayName.trim().charAt(0).toUpperCase() || "?"}
    </span>
  );
}
