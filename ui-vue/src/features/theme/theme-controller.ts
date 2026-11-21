import { t } from "../i18n/locale-service.js";
// Vue Preview supports four deliberate, accessible presets. The legacy
// custom-theme key is intentionally left untouched for reversible migration.
export const THEME_KEY = "pgxray.theme";
export const DEFAULT_THEME = "dba";
export const THEME_CHOICES = [
  { key: "dba", label: "DBA Professional", get description() { return t('tsFinish.themeDbaDesc'); } },
  { key: "ubuntu", label: "Ubuntu Style", get description() { return t('tsFinish.themeUbuntuDesc'); } },
  { key: "light", get label() { return t('tsFinish.themeLight'); }, get description() { return t('tsFinish.themeLightDesc'); } },
  { key: "dark", get label() { return t('tsFinish.themeDark'); }, get description() { return t('tsFinish.themeDarkDesc'); } },
] as const;
export type ThemeName = (typeof THEME_CHOICES)[number]["key"];
export interface ThemeState {
  selected: ThemeName;
  storageError: string | null;
}
export interface ThemeEnvironment {
  storage: Pick<Storage, "getItem" | "setItem">;
  root: {
    dataset: { theme?: string };
    style: Pick<CSSStyleDeclaration, "removeProperty">;
  };
  notifyChanged: (theme: ThemeName) => void;
}

// Remove only the known inline properties set by the previous Custom editor.
// Never erase the saved custom colors: an older frontend may still use them.
const LEGACY_INLINE_PROPERTIES = [
  "--bg",
  "--surface",
  "--surface-2",
  "--surface-3",
  "--border",
  "--border-strong",
  "--border-soft",
  "--border-emphasis",
  "--text",
  "--text-strong",
  "--text-secondary",
  "--text-soft",
  "--muted",
  "--muted-2",
  "--code-text",
  "--accent",
  "--accent-contrast",
  "--secondary",
  "--success",
  "--success-border",
  "--success-bg",
  "--message",
  "--danger",
  "--danger-text",
  "--error",
  "--error-strong",
  "--error-focus",
  "--error-border",
  "--error-bg",
  "--nav-bg",
  "--nav-active-bg",
  "--button-primary",
  "--button-primary-text",
  "--button-secondary",
  "--button-secondary-text",
  "--button-danger",
  "--button-danger-text",
  "--input-bg",
  "--input-text",
  "--terminal-bg",
  "--terminal-text",
  "--terminal-cursor",
  "--terminal-selection",
] as const;

export function normalizeThemeName(value: unknown): ThemeName {
  switch (value) {
    case "dba": case "ubuntu": case "light": case "dark": return value;
    case "graphite-teal": case "forest-mint": case "custom": return "dba";
    case "indigo-cyan": return "dark";
    default: return DEFAULT_THEME;
  }
}

export interface TerminalThemeColors {
  background: string;
  foreground: string;
  cursor: string;
  selectionBackground: string;
}
// Keep the established CSS-variable contract for a future Sessions migration.
export function terminalThemeFromProperties(readProperty: (name: string) => string): TerminalThemeColors {
  const read = (key: string, fallback: string) => readProperty(key).trim() || fallback;
  return {
    background: read("--terminal-bg", "#0b111b"),
    foreground: read("--terminal-text", "#e6edf5"),
    cursor: read("--terminal-cursor", "#70d7c0"),
    selectionBackground: read("--terminal-selection", "#314157"),
  };
}

export function createThemeState(): ThemeState {
  return { selected: DEFAULT_THEME, storageError: null };
}

export function createThemeController(state: ThemeState, env: ThemeEnvironment) {
  function apply(): void {
    for (const property of LEGACY_INLINE_PROPERTIES) env.root.style.removeProperty(property);
    env.root.dataset.theme = state.selected;
    env.notifyChanged(state.selected);
  }

  function initialize(): void {
    let stored: string | null = null;
    try { stored = env.storage.getItem(THEME_KEY); } catch { /* default theme */ }
    state.selected = normalizeThemeName(stored);
    state.storageError = null;
    apply();
  }

  function selectTheme(name: ThemeName): void {
    state.selected = normalizeThemeName(name);
    apply();
    try {
      env.storage.setItem(THEME_KEY, state.selected);
      state.storageError = null;
    } catch {
      state.storageError = t('tsFinish.themeStorage');
    }
  }

  return { initialize, selectTheme };
}
