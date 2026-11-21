/** Local UI preferences, not project permissions or PostgreSQL state. */
export type ProjectSort = "name-asc" | "name-desc" | "connections-desc";
export interface SortableProjectGroup {
  id: number | null;
  name: string;
  connections: readonly unknown[];
}

export function sortProjectGroups<T extends SortableProjectGroup>(groups: readonly T[], sort: ProjectSort): T[] {
  const collator = new Intl.Collator("ru", { sensitivity: "base", numeric: true });
  return [...groups].sort((a, b) => {
    // Ungrouped profiles remain at the bottom, even when sorting by count.
    if (a.id === null) return b.id === null ? 0 : 1;
    if (b.id === null) return -1;
    if (sort === "connections-desc" && a.connections.length !== b.connections.length)
      return b.connections.length - a.connections.length;
    const byName = collator.compare(a.name, b.name);
    return (sort === "name-desc" ? -byName : byName) || a.id - b.id;
  });
}

export interface ProjectViewStorage {
  getItem(key: string): string | null;
  setItem(key: string, value: string): void;
}
function storageKey(username: string): string {
  return `pgxray:project-open:v1:${encodeURIComponent(username)}`;
}
/** Persist IDs only, never connection names, hostnames, credentials or SQL. */
export function parsePinnedProjectIds(raw: string | null): number[] {
  if (!raw) return [];
  try {
    const parsed: unknown = JSON.parse(raw);
    if (!Array.isArray(parsed) || parsed.length > 1000) return [];
    return [...new Set(parsed.filter((id): id is number => Number.isSafeInteger(id) && id > 0))];
  } catch { return []; }
}
export function loadPinnedProjectIds(storage: ProjectViewStorage | null, username: string | null): number[] {
  if (!storage || !username) return [];
  try { return parsePinnedProjectIds(storage.getItem(storageKey(username))); }
  catch { return []; }
}
export function savePinnedProjectIds(storage: ProjectViewStorage | null, username: string | null, ids: readonly number[]): boolean {
  if (!storage || !username) return false;
  try {
    storage.setItem(storageKey(username), JSON.stringify(parsePinnedProjectIds(JSON.stringify(ids))));
    return true;
  } catch { return false; }
}
