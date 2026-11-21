import { t } from "../i18n/locale-service.js";
import type { PgXrayInvoke } from "../../api/invoke.js";
import { isCurrentAuthGeneration, type ShellState } from "../../core/shell-state.js";

export interface CaseArtifact {
  id: number;
  case_id: number;
  original_name: string;
  kind: string;
  visibility: string;
  relative_path: string;
  size_bytes: number;
  created_at_ms: number;
  modified_at_ms: number | null;
}
export type ArtifactRole = "query" | "ddl" | "plan" | "reference" | "note" | "other";
export function isEditableSource(context: ArtifactContext | null): boolean {
  if (!context || context.derivation || context.artifact.visibility !== "sensitive") return false;
  const artifact = context.artifact;
  return artifact.relative_path.split("/").includes("source")
    && ["sql", "text", "source", "import", "explain"].includes(artifact.kind);
}
export function canOpenPlan(context: ArtifactContext | null): boolean {
  if (!context || context.artifact.visibility !== "sensitive") return false;
  const artifact = context.artifact;
  return artifact.kind === "explain" || artifact.original_name.toLowerCase().endsWith(".json");
}
export const artifactRoles: ArtifactRole[] = ["query", "ddl", "plan", "reference", "note", "other"];
// UI preview must not allocate/render the full 32 MiB accepted by the backend.
export const MAX_ARTIFACT_PREVIEW_BYTES = 2 * 1024 * 1024;
export interface ArtifactContext {
  artifact: CaseArtifact;
  role: ArtifactRole | null;
  derivation: { source_artifact_id: number } | null;
  stale: boolean;
}
export interface ArtifactsState {
  caseId: number | null;
  rows: ArtifactContext[];
  selectedId: number | null;
  preview: string | null;
  loading: boolean;
  saving: boolean;
  error: string;
  notice: string;
}
export function createArtifactsState(): ArtifactsState {
  return { caseId: null, rows: [], selectedId: null, preview: null, loading: false, saving: false, error: "", notice: "" };
}
export function createArtifactsController(shell: ShellState, state: ArtifactsState, invoke: PgXrayInvoke) {
  let revision = 0;
  let previewRevision = 0;
  function valid(id: number, generation: number, caseId: number): boolean {
    return revision === id && state.caseId === caseId && isCurrentAuthGeneration(shell, generation);
  }
  function reset(caseId: number | null = null): void {
    ++revision;
    ++previewRevision;
    Object.assign(state, createArtifactsState(), { caseId });
  }
  async function refresh(scanInbox = false, announce = false): Promise<void> {
    const caseId = state.caseId;
    if (caseId === null) return;
    const id = ++revision;
    const generation = shell.authGeneration;
    state.loading = true;
    state.error = "";
    if (announce) state.notice = "";
    try {
      if (scanInbox) await invoke("list_case_artifacts", { caseId, scanInbox: true });
      const context = await invoke<{ artifacts: ArtifactContext[] }>("case_context", { caseId });
      if (!valid(id, generation, caseId)) return;
      state.rows = context.artifacts;
      const retainedId = state.selectedId;
      if (!state.rows.some(row => row.artifact.id === state.selectedId)) {
        state.selectedId = null;
        state.preview = null;
        ++previewRevision;
      } else if (retainedId !== null) {
        await select(retainedId);
      }
      if (announce && valid(id, generation, caseId)) state.notice = scanInbox ? t('tsFinish.toolsInboxScanned') : t('tsFinish.toolsFileListUpdated');
    } catch (error) {
      if (valid(id, generation, caseId)) state.error = String(error);
    } finally {
      if (valid(id, generation, caseId)) state.loading = false;
    }
  }
  async function select(id: number | null): Promise<void> {
    const request = ++previewRevision;
    const generation = shell.authGeneration;
    const caseId = state.caseId;
    state.selectedId = id;
    state.preview = null;
    state.error = "";
    if (id === null || caseId === null) return;
    const selected = state.rows.find(row => row.artifact.id === id && row.artifact.case_id === caseId);
    if (!selected) return;
    if (selected.artifact.size_bytes > MAX_ARTIFACT_PREVIEW_BYTES) {
      state.error = t('tsFinish.artifactTooLarge', { file: selected.artifact.original_name, bytes: String(selected.artifact.size_bytes) });
      return;
    }
    try {
      const content = await invoke<string>("read_case_artifact", { artifactId: id });
      if (request === previewRevision && state.caseId === caseId && state.selectedId === id && isCurrentAuthGeneration(shell, generation)) state.preview = content;
    } catch (error) {
      if (request === previewRevision && state.caseId === caseId && state.selectedId === id && isCurrentAuthGeneration(shell, generation)) state.error = String(error);
    }
  }
  async function mutate(work: () => Promise<unknown>, notice: string): Promise<boolean> {
    const caseId = state.caseId;
    if (caseId === null || state.saving) return false;
    const generation = shell.authGeneration;
    state.saving = true;
    state.error = "";
    state.notice = "";
    try {
      await work();
      if (state.caseId === caseId && isCurrentAuthGeneration(shell, generation)) {
        state.notice = notice;
        await refresh();
      }
      return true;
    } catch (error) {
      if (state.caseId === caseId && isCurrentAuthGeneration(shell, generation)) state.error = String(error);
      return false;
    } finally {
      state.saving = false;
    }
  }
  function createFile(name: string, content: string, kind: string): Promise<boolean> {
    const caseId = state.caseId;
    if (caseId === null || !name.trim()) return Promise.resolve(false);
    return mutate(() => invoke("create_case_file", { caseId, fileName: name.trim(), content, kind }), t('tsFinish.toolsFileCreated'));
  }
  function importFile(sourcePath: string, kind: string): Promise<boolean> {
    const caseId = state.caseId;
    if (caseId === null || !sourcePath.trim()) return Promise.resolve(false);
    return mutate(() => invoke("import_case_file", { caseId, sourcePath: sourcePath.trim(), kind }), t('tsFinish.toolsFileImported'));
  }
  type CreationStatus = "success" | "partial" | "failed";
  async function createWithRole(
    command: "create_case_file" | "import_case_file",
    input: { fileName: string; content: string; kind: string } | { sourcePath: string; kind: string },
    role: ArtifactRole | null,
  ): Promise<CreationStatus> {
    const caseId = state.caseId;
    if (caseId === null || state.saving) return "failed";
    const generation = shell.authGeneration;
    state.saving = true;
    state.error = "";
    state.notice = "";
    let result: CreationStatus = "failed";
    let roleFailure = "";
    try {
      const created = await invoke<CaseArtifact>(command, { caseId, ...input });
      result = "success";
      if (role !== null) {
        try {
          await invoke("set_case_artifact_role", { artifactId: created.id, role });
        } catch (roleError) {
          result = "partial";
          roleFailure = t('tsFinish.artifactRoleFailure', { id: String(created.id), error: String(roleError) });
        }
      }
      if (result === "success" && state.caseId === caseId && isCurrentAuthGeneration(shell, generation)) {
        state.notice = command === "import_case_file" ? t('tsFinish.toolsFileImported') : t('tsFinish.toolsFileCreated');
      }
    } catch (error) {
      if (state.caseId === caseId && isCurrentAuthGeneration(shell, generation)) state.error = String(error);
    } finally {
      state.saving = false;
      if (result !== "failed" && state.caseId === caseId && isCurrentAuthGeneration(shell, generation)) {
        await refresh();
        if (roleFailure && state.caseId === caseId && isCurrentAuthGeneration(shell, generation)) state.error = roleFailure;
      }
    }
    return result;
  }
  function createFileWithRole(name: string, content: string, kind: string, role: ArtifactRole | null): Promise<CreationStatus> {
    if (!name.trim()) return Promise.resolve("failed");
    return createWithRole("create_case_file", { fileName: name.trim(), content, kind }, role);
  }
  function importFileWithRole(sourcePath: string, kind: string, role: ArtifactRole | null): Promise<CreationStatus> {
    if (!sourcePath.trim()) return Promise.resolve("failed");
    return createWithRole("import_case_file", { sourcePath: sourcePath.trim(), kind }, role);
  }
  async function updateFile(artifactId: number, content: string): Promise<boolean> {
    const context = state.rows.find(row => row.artifact.id === artifactId) ?? null;
    if (!isEditableSource(context) || state.selectedId !== artifactId) {
      state.error = t('tsFinish.toolsLocalOnly');
      return false;
    }
    if (new TextEncoder().encode(content).length > 2 * 1024 * 1024) {
      state.error = t('tsFinish.toolsEditorSize');
      return false;
    }
    const ok = await mutate(
      () => invoke("update_case_file", { artifactId, content }),
      t('tsFinish.toolsFileSaved'),
    );
    if (ok && state.selectedId === artifactId) await select(artifactId);
    return ok;
  }
  async function exportFile(artifactId: number): Promise<void> {
    const context = state.rows.find(row => row.artifact.id === artifactId);
    if (!context || state.saving) return;
    const caseId = state.caseId;
    const generation = shell.authGeneration;
    state.saving = true;
    state.error = "";
    try {
      const path = await invoke<string>("export_case_artifact", { artifactId });
      if (state.caseId === caseId && isCurrentAuthGeneration(shell, generation)) state.notice = t('tsFinish.artifactExported', { path });
    } catch (error) {
      if (state.caseId === caseId && isCurrentAuthGeneration(shell, generation)) state.error = String(error);
    } finally { state.saving = false; }
  }
  async function openPlan(artifactId: number): Promise<void> {
    const context = state.rows.find(row => row.artifact.id === artifactId) ?? null;
    if (!canOpenPlan(context) || state.caseId === null) return;
    const caseId = state.caseId;
    const generation = shell.authGeneration;
    state.error = "";
    try {
      await invoke("open_plan_workspace", { caseId, artifactId });
      if (state.caseId === caseId && isCurrentAuthGeneration(shell, generation)) state.notice = t('tsFinish.toolsPlanOpened');
    } catch (error) {
      if (state.caseId === caseId && isCurrentAuthGeneration(shell, generation)) state.error = String(error);
    }
  }
  async function revealFile(artifactId: number): Promise<void> {
    if (!state.rows.some(row => row.artifact.id === artifactId) || state.saving) return;
    state.error = "";
    try {
      const path = await invoke<string>("reveal_case_artifact", { artifactId });
      if (state.rows.some(row => row.artifact.id === artifactId)) state.notice = t('tsFinish.artifactFolder', { path });
    } catch (error) { state.error = String(error); }
  }
  async function copyFile(artifactId: number): Promise<void> {
    if (!state.rows.some(row => row.artifact.id === artifactId) || state.saving) return;
    state.error = "";
    try {
      const text = await invoke<string>("read_case_artifact", { artifactId });
      if (!state.rows.some(row => row.artifact.id === artifactId)) return;
      await navigator.clipboard.writeText(text);
      state.notice = t('tsFinish.toolsCopied');
    } catch (error) { state.error = String(error); }
  }
  function anonymizeFile(artifactId: number): Promise<boolean> {
    const caseId = state.caseId;
    const item = state.rows.find(row => row.artifact.id === artifactId);
    if (caseId === null || !item || item.artifact.visibility !== "sensitive") return Promise.resolve(false);
    const name = item.artifact.original_name.toLowerCase();
    const mode = item.artifact.kind === "explain" ? "explain"
      : item.artifact.kind === "sql" || name.endsWith(".sql") ? "sql" : "result";
    return mutate(() => invoke("anonymize_case_artifact", { caseId, artifactId, mode }), t('tsFinish.toolsShareable'));
  }
  function setRole(artifactId: number, role: ArtifactRole | null): Promise<boolean> {
    if (!state.rows.some(row => row.artifact.id === artifactId)) return Promise.resolve(false);
    return mutate(() => invoke("set_case_artifact_role", { artifactId, role }), t('tsFinish.toolsRoleUpdated'));
  }
  async function deleteArtifact(artifactId: number): Promise<boolean> {
    if (!state.rows.some(row => row.artifact.id === artifactId)) return false;
    const ok = await mutate(() => invoke("delete_case_artifact", { artifactId, deleteComparisons: false }), t('tsFinish.toolsArtifactDeleted'));
    if (ok && state.selectedId === artifactId) { state.selectedId = null; state.preview = null; ++previewRevision; }
    return ok;
  }
  return { reset, refresh, select, createFile, importFile, createFileWithRole, importFileWithRole, updateFile, exportFile, openPlan, revealFile, copyFile, anonymizeFile, setRole, deleteArtifact };
}
