export interface ProjectFileCommitImpactCounts {
  add: number;
  replace: number;
  deleteFile: number;
  deleteDirectory: number;
}

interface Props {
  counts: ProjectFileCommitImpactCounts;
  deletionTarget: "local" | "storage";
  deletionApproved: boolean;
  disabled: boolean;
  onDeletionApprovedChange: (approved: boolean) => void;
}

export function projectFileDeletionCount(counts: ProjectFileCommitImpactCounts): number {
  return counts.deleteFile + counts.deleteDirectory;
}

export default function ProjectFileCommitImpact({
  counts,
  deletionTarget,
  deletionApproved,
  disabled,
  onDeletionApprovedChange,
}: Props) {
  const total = counts.add + counts.replace + projectFileDeletionCount(counts);
  if (total === 0) return null;

  const deletionCount = projectFileDeletionCount(counts);
  const deletionCopy = deletionTarget === "storage"
    ? `Approve removal of ${counts.deleteFile} file${counts.deleteFile === 1 ? "" : "s"} and ${counts.deleteDirectory} folder${counts.deleteDirectory === 1 ? "" : "s"} from storage.`
    : `Approve deletion of ${counts.deleteFile} local file${counts.deleteFile === 1 ? "" : "s"} and ${counts.deleteDirectory} empty local folder${counts.deleteDirectory === 1 ? "" : "s"}. Files are backed up.`;

  /* Only the operations this commit performs: a zero says nothing, and four
     of them took the footer's room from the buttons. */
  const operations: [number, string][] = [
    [counts.add, "Add"],
    [counts.replace, "Replace"],
    [counts.deleteFile, "Delete files"],
    [counts.deleteDirectory, "Delete folders"],
  ];

  return (
    <div
      className={`sync-project-file-commit-impact${deletionCount > 0 ? " with-approval" : ""}`}
      aria-label="Selected project file impact"
    >
      <div className="sync-project-file-impact-counts">
        {operations.filter(([count]) => count > 0).map(([count, label]) => (
          <span key={label}><strong>{count}</strong>{` ${label}`}</span>
        ))}
      </div>
      {deletionCount > 0 && (
        <label className="sync-project-file-deletion-ack">
          <span className="sync-sync-row-choice">
            <input
              type="checkbox"
              checked={deletionApproved}
              disabled={disabled}
              aria-label="Approve the selected project file deletions"
              title="Approve the selected project file deletions"
              onChange={(event) => onDeletionApprovedChange(event.target.checked)}
            />
          </span>
          <span>{deletionCopy}</span>
        </label>
      )}
    </div>
  );
}
