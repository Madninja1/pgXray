"use strict";
// Legacy HTML WebView also needs the native browser bridge.
(() => {
  const rawInvoke = window.__TAURI__?.core?.invoke;
  if (typeof rawInvoke !== "function") return;
  document.addEventListener("click", (event) => {
    if (event.defaultPrevented || event.button !== 0 || !(event.target instanceof Element)) return;
    const anchor = event.target.closest("a[href]");
    if (!anchor) return;
    const href = anchor.getAttribute("href");
    if (!href || !/^https?:\/\//i.test(href)) return;
    event.preventDefault();
    let url;
    try {
      url = new URL(href);
      if (!["http:", "https:"].includes(url.protocol) || !url.hostname || url.username || url.password)
        throw new Error("Недопустимый адрес");
    } catch (error) {
      window.alert(`Не удалось открыть ссылку: ${String(error)}`);
      return;
    }
    void rawInvoke("open_external_url", { url: url.href })
      .catch(error => window.alert(`Не удалось открыть ссылку в браузере: ${String(error)}`));
  }, true);
})();
