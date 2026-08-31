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

  return (
    <div className="sync-project-file-commit-impact" aria-label="Selected project file impact">
      <div className="sync-project-file-impact-counts">
        <span><strong>{counts.add}</strong> Add</span>
        <span><strong>{counts.replace}</strong> Replace</span>
        <span><strong>{counts.deleteFile}</strong> Delete files</span>
        <span><strong>{counts.deleteDirectory}</strong> Delete folders</span>
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
