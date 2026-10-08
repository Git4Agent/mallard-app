/** Words every area shares: verbs and states. Owned by the i18n foundation;
 *  an area that needs another shared word keeps it in its own namespace. */
export default {
  push: "Push",
  pull: "Pull",
  cancel: "Cancel",
  close: "Close",
  save: "Save",
  delete: "Delete",
  remove: "Remove",
  open: "Open",
  back: "Back",
  tryAgain: "Try again",
  refresh: "Refresh",
  copy: "Copy",
  copied: "Copied",
  /** What macOS draws on a dialog button that names none. */
  ok: "OK",
  settings: "Settings",
  loading: "Loading…",
  saving: "Saving…",
  saved: "Saved",
  notSaved: "Not saved",
  version: "Version {{version}}",
  /** A Push that finished an earlier unfinished one. */
  pushFinished: "Finished. Compare again.",
} as const;
