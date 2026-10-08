import type { ParseKeys } from "i18next";
import { useTranslation } from "../../i18n";

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
  const { t } = useTranslation("projects");
  const total = counts.add + counts.replace + projectFileDeletionCount(counts);
  if (total === 0) return null;

  const deletionCount = projectFileDeletionCount(counts);
  const deletionCopy = deletionTarget === "storage"
    ? t("impact.approveRemoval", {
      files: t("impact.files", { count: counts.deleteFile }),
      folders: t("impact.folders", { count: counts.deleteDirectory }),
    })
    : t("impact.approveDeletion", {
      files: t("impact.localFiles", { count: counts.deleteFile }),
      folders: t("impact.emptyFolders", { count: counts.deleteDirectory }),
    });

  /* Only the operations this commit performs: a zero says nothing, and four
     of them took the footer's room from the buttons. */
  const operations: [number, ParseKeys<"projects">][] = [
    [counts.add, "impact.add"],
    [counts.replace, "impact.replace"],
    [counts.deleteFile, "impact.deleteFiles"],
    [counts.deleteDirectory, "impact.deleteFolders"],
  ];

  return (
    <div
      className={`sync-project-file-commit-impact${deletionCount > 0 ? " with-approval" : ""}`}
      aria-label={t("impact.label")}
    >
      <div className="sync-project-file-impact-counts">
        {operations.filter(([count]) => count > 0).map(([count, label]) => (
          <span key={label}><strong>{count}</strong>{` ${t(label)}`}</span>
        ))}
      </div>
      {deletionCount > 0 && (
        <label className="sync-project-file-deletion-ack">
          <span className="sync-sync-row-choice">
            <input
              type="checkbox"
              checked={deletionApproved}
              disabled={disabled}
              aria-label={t("impact.approve")}
              title={t("impact.approve")}
              onChange={(event) => onDeletionApprovedChange(event.target.checked)}
            />
          </span>
          <span>{deletionCopy}</span>
        </label>
      )}
    </div>
  );
}
