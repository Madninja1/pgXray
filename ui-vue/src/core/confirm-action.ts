import { t } from "../features/i18n/locale-service.js";

/** Webview-safe, explicit confirmation. Never approve an action if the dialog is dismissed. */
export function confirmAction(message: string): Promise<boolean> {
  return new Promise(resolve => {
    const dialog = document.createElement("dialog");
    dialog.setAttribute("aria-label", t("investigation.confirmTitle"));
    dialog.style.cssText = "max-width:min(32rem,calc(100vw - 2rem));min-width:min(20rem,calc(100vw - 2rem));padding:1.25rem;border:1px solid var(--border, #78677d);border-radius:0.65rem;background:var(--panel, #302238);color:var(--text, #fff);box-shadow:0 1rem 3rem #0008";
    const text = document.createElement("p");
    text.textContent = message;
    text.style.cssText = "white-space:pre-wrap;overflow-wrap:anywhere;margin:0 0 1.25rem";
    const actions = document.createElement("div");
    actions.style.cssText = "display:flex;justify-content:flex-end;gap:0.75rem";
    const cancel = document.createElement("button");
    cancel.type = "button";
    cancel.textContent = t("investigation.cancel");
    const approve = document.createElement("button");
    approve.type = "button";
    approve.textContent = t("investigation.confirm");
    cancel.addEventListener("click", () => dialog.close("cancel"));
    approve.addEventListener("click", () => dialog.close("approve"));
    actions.append(cancel, approve);
    dialog.append(text, actions);
    dialog.addEventListener("close", () => {
      const accepted = dialog.returnValue === "approve";
      dialog.remove();
      resolve(accepted);
    }, { once: true });
    document.body.append(dialog);
    try { dialog.showModal(); cancel.focus(); }
    catch { dialog.remove(); resolve(false); }
  });
}
