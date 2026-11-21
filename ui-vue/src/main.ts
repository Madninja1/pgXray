import "../../ui/styles/tokens.css";
import "../../ui/vendor/xterm/xterm.css";
import "./features/theme/presets.css";
import { createApp, watch } from "vue";
import { initializeTheme } from "./features/theme/theme-service.js";
import { initializeLocale, locale } from "./features/i18n/locale-service.js";
import App from "./App.vue";
import { invoke, isTauri } from "@tauri-apps/api/core";
import { installExternalLinkHandler } from "./core/external-links.js";

initializeTheme();
initializeLocale();
// Rust diagnostics are produced on demand. Initialize their locale before mounting
// so the first collection uses the same language as the selected UI.
async function mountApplication(): Promise<void> {
  if (isTauri()) {
    try {
      await invoke("set_backend_locale", { locale: locale.value });
    } catch (error) {
      console.error("Could not initialize backend locale", error);
    }
    watch(locale, value => {
      void invoke("set_backend_locale", { locale: value }).catch(error =>
        console.error("Could not update backend locale", error));
    });
  }
  createApp(App).mount("#app");
}
void mountApplication();

// Install once for every Vue feature (including future ones). In a normal
// browser/Vite preview links retain native navigation behavior.
if (isTauri()) {
  installExternalLinkHandler(document,
    (url) => invoke("open_external_url", { url }),
    (message) => window.alert(message));
}
