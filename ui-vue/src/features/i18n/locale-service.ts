import { ref } from "vue";
import en from "./locales/en.json" with { type: "json" };
import ru from "./locales/ru.json" with { type: "json" };

export type AppLocale = "en" | "ru";
export const LOCALE_STORAGE_KEY = "pgxray.locale";

const messages = { en, ru } as const;
export const locale = ref<AppLocale>("en");
export const localeStorageError = ref(false);

function setDocumentLanguage(value: AppLocale): void {
  document.documentElement.lang = value;
}

/** The preference stays on this device; only its en/ru code is sent to the bundled Rust backend, never to PostgreSQL. */
export function initializeLocale(): void {
  let stored: string | null = null;
  try { stored = window.localStorage.getItem(LOCALE_STORAGE_KEY); }
  catch { /* Private browsing or disabled storage: use English. */ }
  locale.value = stored === "ru" ? "ru" : "en";
  localeStorageError.value = false;
  setDocumentLanguage(locale.value);
}

export function selectLocale(value: AppLocale): void {
  locale.value = value;
  setDocumentLanguage(value);
  try {
    window.localStorage.setItem(LOCALE_STORAGE_KEY, value);
    localeStorageError.value = false;
  } catch {
    localeStorageError.value = true;
  }
}

type MessageGroup = keyof typeof en;
type MessageKey = {
  [G in MessageGroup]: `${G}.${Extract<keyof (typeof en)[G], string>}`;
}[MessageGroup];

/** Reads the reactive locale during Vue rendering, so changing it updates labels. */
export function t(key: MessageKey, params: Record<string, string> = {}): string {
  const [group, name] = key.split(".") as [MessageGroup, string];
  const dictionary = messages[locale.value] as Record<string, Record<string, string>>;
  const template = dictionary[group]?.[name] ?? key;
  return template.replace(/\{(\w+)\}/g, (_, name: string) => params[name] ?? `{${name}}`);
}
