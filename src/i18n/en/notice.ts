/** Notices and acknowledgements (notice/). */
export default {
  /** The rung's own word, leading every message: "Error ·". */
  severity: {
    danger: "Error",
    warning: "Warning",
    info: "Note",
    success: "Done",
  },
  dismiss: "Dismiss",
  /** The close control's accessible name; `what` is the message it closes. */
  dismissWhat: "Dismiss: {{what}}",
  /** A message as one spoken line: a part that does not end as a sentence
   *  takes this stop. */
  spoken: {
    stop: "{{part}}.",
  },
  /** A zone's count line. The caller's subject carries its own noun —
   *  in Chinese, its measure word first. */
  zone: {
    count: "{{count}} {{subject}}",
    all: "all {{count}} {{subject}}",
  },
  /** The message under an invalid field. */
  field: {
    spoken: "Error: ",
    dismiss: "Error. {{text}}",
  },
} as const;
