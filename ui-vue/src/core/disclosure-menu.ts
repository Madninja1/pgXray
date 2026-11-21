/** Close native <details> menus on a pointer event outside their own content.
 * Callers attach/remove the document listener in the component lifecycle. */
export function dismissOpenDetailsOutside(
  container: ParentNode | null,
  selector: string,
  target: EventTarget | null,
): void {
  if (!container || !target || typeof target !== "object") return;
  container.querySelectorAll<HTMLDetailsElement>(selector).forEach(menu => {
    if (menu.open && !menu.contains(target as Node)) menu.open = false;
  });
}

export function closeOpenDetails(container: ParentNode | null, selector: string): void {
  container?.querySelectorAll<HTMLDetailsElement>(selector).forEach(menu => { menu.open = false; });
}
