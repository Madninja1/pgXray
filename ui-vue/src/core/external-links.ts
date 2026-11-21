import { t } from "../features/i18n/locale-service.js";
/** Hand off documentation links to the system browser in Tauri WebView. */
export function safeExternalUrl(value: string): string | null {
  try {
    const url = new URL(value);
    if ((url.protocol !== "https:" && url.protocol !== "http:") || !url.hostname ||
        url.username || url.password) return null;
    return url.href;
  } catch { return null; }
}

export function installExternalLinkHandler(
  root: Document,
  open: (url: string) => Promise<unknown>,
  onError: (message: string) => void,
): () => void {
  const handle = (event: MouseEvent) => {
    if (event.defaultPrevented || event.button !== 0 || !(event.target instanceof Element)) return;
    const anchor = event.target.closest("a[href]");
    if (!anchor) return;
    const raw = anchor.getAttribute("href");
    if (!raw || !/^https?:\/\//i.test(raw)) return;
    event.preventDefault();
    const url = safeExternalUrl(raw);
    if (!url) { onError(t('tsFinish.linkInvalid')); return; }
    void open(url).catch(error => onError(t('tsFinish.linkOpenFailed', { error: String(error) })));
  };
  root.addEventListener("click", handle, true);
  return () => root.removeEventListener("click", handle, true);
}
