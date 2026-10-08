/** The Logs drawer (LogPanel, ActivityLogDrawer). The lines themselves are
 *  the backend's and stay English. */
export default {
  drawer: {
    label: "Logs",
    resize: "Resize log",
  },
  /** The type filter's options. */
  types: {
    repair: "Repair",
    storage: "Storage",
    configuration: "Configuration",
    history: "History",
    system: "System",
  },
  /** The tag on each line: the type's own word, lowercase in English. */
  tag: {
    push: "push",
    pull: "pull",
    repair: "repair",
    storage: "storage",
    configuration: "configuration",
    history: "history",
    system: "system",
  },
  levels: {
    info: "Info",
    success: "Success",
    warning: "Warning",
    error: "Error",
  },
  panel: {
    title: "log",
    typeFilter: "Filter log by types: {{selection}}",
    type: "Type",
    types: "Log types",
    allTypes: "All",
    noTypes: "None",
    typesSelected: "{{count}} selected",
    selectAll: "Select all",
    deselectAll: "Deselect all",
    level: "Level",
    levelFilter: "Filter log by level",
    allLevels: "All",
    searchPlaceholder: "Search logs",
    search: "Search log",
    manage: "Manage logs",
    clear: "clear",
    clearLabel: "Clear log",
    closeLabel: "Close log",
    loadOlder: "Load older logs",
    loadFailed: "Could not load retained logs: {{error}}",
    loadingRetained: "Loading retained logs…",
    noMatches: "No entries match these filters.",
  },
} as const;
