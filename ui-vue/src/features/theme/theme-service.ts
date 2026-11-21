import { reactive } from "vue";
import { createThemeController, createThemeState, terminalThemeFromProperties, type ThemeEnvironment } from "./theme-controller.js";

export const themeState = reactive(createThemeState());

const environment: ThemeEnvironment = {
  root: document.documentElement,
  storage: {
    getItem(key) { return window.localStorage.getItem(key); },
    setItem(key, value) { window.localStorage.setItem(key, value); },
  },
  notifyChanged(theme) {
    window.dispatchEvent(new CustomEvent("pgxray-theme-changed", { detail: { theme } }));
  },
};
export const themeController = createThemeController(themeState, environment);
let initialized = false;

export function initializeTheme(): void {
  if (initialized) return;
  initialized = true;
  themeController.initialize();
}

// Public for the Sessions module when it is migrated; no terminal instance yet exists in the preview.
export function terminalThemeFromCss() {
  const computed = getComputedStyle(document.documentElement);
  return terminalThemeFromProperties(name => computed.getPropertyValue(name));
}
