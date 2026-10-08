/** Shared review furniture: path bar, selection bar, tabs, search, outcome notices (sync-review/). */
export default {
  /** The workspace header (SyncPathBar). `label` is the menu's own noun,
   *  lowercased in English: "Switch storage…". */
  pathBar: {
    label: "Workspace path",
    more: "More actions",
    switch: "Switch",
    switchPlaceholder: "Switch {{label}}…",
    filter: "Filter {{label}} list",
    noMatch: "No match.",
    messagesLabel: "Messages from Mallard",
    /** The banner zone's count subject: "3 messages from Mallard". */
    messages: "messages from Mallard",
  },
  /** A page's way back (PageBack). */
  back: "Back to {{label}}",
  /** The search row over a list (NameSearchField). `noun` is the caller's
   *  counted noun for `total` — "files", or 个文件 in Chinese. */
  search: {
    placeholder: "{{label}}…",
    clear: "Clear search",
    total: "{{total}} {{noun}}",
    matches: "<b>{{matches}}</b><span> of {{total}} {{noun}}</span>",
    matchesSpoken: "{{matches}} of {{total}} {{noun}}",
    hiddenSelected: "{{count}} selected hidden by search",
  },
  /** The status scope bar and the five scopes a project list splits into. */
  scopes: {
    all: "All",
    /** A scope's accessible name; `noun` is counted for `count`. */
    option: "{{name}}, {{count}} {{noun}}",
    push: "To push",
    pull: "To pull",
    attention: "Attention",
    synced: "Up to date",
    unknown: "Not compared",
  },
  /** A review header's selection cluster (SyncReviewSelectionBar). */
  selection: {
    count: "{{count}} of {{total}} selected",
    select: "Select <n>{{count}}</n>",
    unselect: "Unselect <n>{{count}}</n>",
    clearMatching: "Clear matching selection",
    clearCurrent: "Clear current selection",
  },
  tabs: {
    label: "Sync review steps",
    tab: "{{label}}, {{count}} selected",
    tabWarning: "{{label}}, {{count}} selected, needs attention",
  },
  /** What a review's selection moves against the most one apply may. */
  budget: {
    pushSize: "Push size",
    pullSize: "Pull size",
    overPush: "{{over}} over the {{limit}} push limit — exclude sessions or files to push.",
    overPull: "{{over}} over the {{limit}} pull limit — exclude sessions or files to pull.",
    fitsPush: "This push · {{total}} of {{limit}}",
    fitsPull: "This pull · {{total}} of {{limit}}",
  },
  /** The review's frame while it is prepared (ReviewPreparing). */
  preparing: {
    label: "Preparing…",
    cancel: "Cancel — nothing has changed",
    writing: "Changes are being written; this can't be stopped now",
    cancelLabel: "Cancel preparing the {{title}}",
  },
  /** A finished sync on its storage card (SyncOutcomeNotice). */
  outcome: {
    synced: "Synced",
    held: "Held for Pull",
    preserved: "Kept independently",
    failed: "Failed",
    notAttempted: "Not attempted",
    skipped: "Skipped",
    notPulled: "Not pulled",
    /** A row's status before the backend's reason for it. */
    rowStatus: "{{status}}. {{reason}}",
    /** The close control's name: the rung's word, then the title. */
    dismiss: "{{word}}. {{title}}",
    collapse: "Collapse the result list",
    expand: "Expand the result list",
    technical: "Technical results",
  },
  clipboard: {
    unavailable: "Clipboard access is unavailable",
  },
  /** A Markdown image, track or clip that is not drawn inline
   *  (MarkdownImagePlaceholder). The transcript around it is never copy. */
  media: {
    image: "image",
    audio: "audio",
    video: "video",
    loading: "loading…",
    notShown: "not shown",
    name: "{{label}} — {{kind}} {{state}}",
    nameDetail: "{{label}} — {{kind}} {{state}}. {{detail}}",
    nameReason: "{{label}} — {{reason}}",
  },
} as const;
