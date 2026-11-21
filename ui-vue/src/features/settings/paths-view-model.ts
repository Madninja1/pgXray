/** UI-only path draft checks. The Rust backend remains authoritative for
 * filesystem permissions, symlinks, validation, and atomic migrations. */
export type PathKind = "export" | "workspace" | "log";
export type PathDraftStatus = "empty" | "unchanged" | "changed";

export function pathDraftStatus(draft: string, current: string): PathDraftStatus {
  const trimmed = draft.trim();
  if (!trimmed) return "empty";
  return trimmed === current.trim() ? "unchanged" : "changed";
}
