import type { SavedConnection } from "../features/connections/controller.js";

/** A command, not a persistent global connection: background operations keep their original target. */
export interface QuickConnectionSelection {
  connectionId: number;
  serial: number;
}

/** Returns null when the key belongs to an editor, a dialog, the OS or an unassigned slot. */
export function shortcutForKey(
  event: Pick<KeyboardEvent, "key" | "altKey" | "ctrlKey" | "metaKey" | "shiftKey" | "repeat" | "target">,
  connections: readonly Pick<SavedConnection, "id" | "shortcut_slot">[],
  modalOpen: boolean,
): number | null {
  if (modalOpen || event.repeat || event.altKey || event.ctrlKey || event.metaKey || event.shiftKey || !/^[0-9]$/.test(event.key)) return null;
  const element = event.target;
  if (typeof Element !== "undefined" && element instanceof Element && element.closest('input, textarea, select, [contenteditable], [role="textbox"], .xterm, .cm-editor')) return null;
  return connections.find(connection => connection.shortcut_slot === Number(event.key))?.id ?? null;
}
