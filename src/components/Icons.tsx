import {
  IconActivityHeartbeat,
  IconAdjustmentsHorizontal,
  IconAlertTriangle,
  IconBan,
  IconBrandOpenai,
  IconBulb,
  IconCheck,
  IconChevronDown,
  IconChevronLeft,
  IconChevronRight,
  IconChevronUp,
  IconCircleCheck,
  IconClock,
  IconCloud,
  IconCloudCheck,
  IconCloudExclamation,
  IconCloudLock,
  IconCloudOff,
  IconCloudPause,
  IconCloudPlus,
  IconCloudX,
  IconCode,
  IconClipboardList,
  IconCopy,
  IconDatabase,
  IconDeviceDesktopCog,
  IconDots,
  IconDownload,
  IconEdit,
  IconExternalLink,
  IconFile,
  IconFolder,
  IconFolderCode,
  IconGitBranch,
  IconGitCommit,
  IconGitCompare,
  IconGitFork,
  IconHexagon,
  IconInfoCircle,
  IconLayoutGrid,
  IconLayoutSidebarLeftCollapse,
  IconLayoutSidebarLeftExpand,
  IconLink,
  IconLinkOff,
  IconLock,
  IconMessage,
  IconMoon,
  IconPlug,
  IconPlus,
  IconRefresh,
  IconSearch,
  IconSquareRoundedX,
  IconStar,
  IconStarFilled,
  IconSun,
  IconTerminal2,
  IconTool,
  IconTrash,
  IconUpload,
  IconUser,
  IconUserOff,
  IconX,
  type TablerIcon,
} from "@tabler/icons-react";
import type { SVGProps } from "react";
import CatalogPetIcon from "./CatalogPetIcon";

export type IconName =
  | "activity"
  | "alert-triangle"
  | "ban"
  | "catalog"
  | "catalog-pet-v1"
  | "catalog-pet-v2"
  | "catalog-plugin"
  | "check"
  | "chevron-down"
  | "chevron-left"
  | "chevron-right"
  | "chevron-up"
  | "claude-mark"
  | "clock"
  | "cloud"
  | "cloud-dot"
  | "cloud-check"
  | "cloud-lock"
  | "cloud-clock"
  | "cloud-x"
  | "cloud-alert"
  | "cloud-user"
  | "cloud-off"
  | "code"
  | "computer"
  | "copy"
  | "clipboard-list"
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
  | "plus"
  | "range"
  | "refresh"
  | "settings"
  | "sidebar-collapse"
  | "sidebar-expand"
  | "storage"
  | "storage-local"
  | "storage-mallard"
  | "storage-feishu"
  | "storage-dropbox"
  | "storage-r2"
  | "sun"
  | "token"
  | "tool"
  | "trash"
  | "terminal"
  | "upload"
  | "star"
  | "star-filled"
  | "search"
  | "user"
  | "x";

interface IconProps extends SVGProps<SVGSVGElement> {
  name: IconName;
  size?: number;
}

const HINTED_ICONS: readonly IconName[] = ["cloud", "drive", "folder", "computer", "link", "lock"];

export function iconNameFromHint(hint: string | null | undefined, fallback: IconName): IconName {
  return hint && (HINTED_ICONS as readonly string[]).includes(hint) ? hint as IconName : fallback;
}

const STORAGE_PROVIDER_ICONS = new Map<string, IconName>([
  ["local", "storage-local"],
  ["managed_cloud", "storage-mallard"],
  ["feishu", "storage-feishu"],
  ["dropbox", "storage-dropbox"],
  ["r2", "storage-r2"],
]);

const STORAGE_ICON_ASSETS: Partial<Record<IconName, string>> = {
  "storage-local": "local.svg",
  "storage-mallard": "mallard-cloud.png",
  "storage-feishu": "feishu.svg",
  "storage-dropbox": "dropbox.svg",
  "storage-r2": "r2.svg",
};

export function storageIconName(provider: string | null | undefined, hint?: string | null): IconName {
  return STORAGE_PROVIDER_ICONS.get(provider ?? "") ?? iconNameFromHint(hint, "drive");
}

type CustomIconName =
  | "catalog-pet-v1"
  | "catalog-pet-v2"
  | "claude-mark"
  | "storage-local"
  | "storage-mallard"
  | "storage-feishu"
  | "storage-dropbox"
  | "storage-r2";
type LibraryIconName = Exclude<IconName, CustomIconName>;

const LIBRARY_ICONS: Record<LibraryIconName, TablerIcon> = {
  activity: IconActivityHeartbeat,
  "alert-triangle": IconAlertTriangle,
  ban: IconBan,
  catalog: IconLayoutGrid,
  "catalog-plugin": IconPlug,
  check: IconCheck,
  "chevron-down": IconChevronDown,
  "chevron-left": IconChevronLeft,
  "chevron-right": IconChevronRight,
  "chevron-up": IconChevronUp,
  clock: IconClock,
  cloud: IconCloud,
  "cloud-dot": IconCloudPlus,
  "cloud-check": IconCloudCheck,
  "cloud-lock": IconCloudLock,
  "cloud-clock": IconCloudPause,
  "cloud-x": IconCloudX,
  "cloud-alert": IconCloudExclamation,
  "cloud-user": IconUserOff,
  "cloud-off": IconCloudOff,
  code: IconCode,
  computer: IconDeviceDesktopCog,
  copy: IconCopy,
  "clipboard-list": IconClipboardList,
  "check-circle": IconCircleCheck,
  download: IconDownload,
  drive: IconDatabase,
  edit: IconEdit,
  file: IconFile,
  folder: IconFolder,
  "external-link": IconExternalLink,
  "git-branch": IconGitBranch,
  "git-commit": IconGitCommit,
  "git-compare-arrows": IconGitCompare,
  "git-fork": IconGitFork,
  "git-folder": IconFolderCode,
  info: IconInfoCircle,
  lightbulb: IconBulb,
  link: IconLink,
  "link-off": IconLinkOff,
  lock: IconLock,
  message: IconMessage,
  moon: IconMoon,
  more: IconDots,
  "octagon-x": IconSquareRoundedX,
  openai: IconBrandOpenai,
  plus: IconPlus,
  range: IconAdjustmentsHorizontal,
  refresh: IconRefresh,
  settings: IconAdjustmentsHorizontal,
  "sidebar-collapse": IconLayoutSidebarLeftCollapse,
  "sidebar-expand": IconLayoutSidebarLeftExpand,
  storage: IconDatabase,
  sun: IconSun,
  token: IconHexagon,
  tool: IconTool,
  trash: IconTrash,
  terminal: IconTerminal2,
  upload: IconUpload,
  star: IconStar,
  "star-filled": IconStarFilled,
  search: IconSearch,
  user: IconUser,
  x: IconX,
};

export default function Icon({ name, size = 14, ...props }: IconProps) {
  const storageAsset = STORAGE_ICON_ASSETS[name];
  if (storageAsset) {
    return (
      <svg
        {...props}
        aria-hidden="true"
        data-icon={name}
        data-storage-provider={name.slice("storage-".length)}
        focusable="false"
        fill="none"
        height={size}
        shapeRendering="geometricPrecision"
        viewBox="0 0 24 24"
        width={size}
      >
        <image href={`/storage-provider-icons/${storageAsset}`} width="24" height="24" />
      </svg>
    );
  }

  if (name === "catalog-pet-v1" || name === "catalog-pet-v2") {
    return <CatalogPetIcon {...props} name={name} size={size} />;
  }

  if (name === "claude-mark") {
    return (
      <svg
        {...props}
        aria-hidden="true"
        data-icon={name}
        focusable="false"
        fill="none"
        height={size}
        shapeRendering="geometricPrecision"
        viewBox="0 0 24 24"
        width={size}
      >
        <path
          className="icon-claude-mark"
          fill="currentColor"
          stroke="none"
          d="M12 12L10.1 1.5L13.9 1.5ZM12 12L18.08 3.23L20.77 5.92ZM12 12L22.5 10.1L22.5 13.9ZM12 12L20.77 18.08L18.08 20.77ZM12 12L13.9 22.5L10.1 22.5ZM12 12L5.92 20.77L3.23 18.08ZM12 12L1.5 13.9L1.5 10.1ZM12 12L3.23 5.92L5.92 3.23Z"
        />
      </svg>
    );
  }

  const LibraryIcon = LIBRARY_ICONS[name as LibraryIconName];
  return (
    <LibraryIcon
      {...props}
      aria-hidden="true"
      data-icon={name}
      focusable="false"
      shapeRendering="geometricPrecision"
      size={size}
      stroke={2}
    />
  );
}
