<script setup lang="ts">
import { computed } from "vue";
import { THEME_CHOICES, type ThemeName } from "./theme-controller.js";
import { themeController, themeState } from "./theme-service.js";
import { locale, localeStorageError, selectLocale, t, type AppLocale } from "../i18n/locale-service.js";

const currentLabel = computed(() =>
  THEME_CHOICES.find(({ key }) => key === themeState.selected)?.label ?? "DBA Professional",
);
function select(theme: ThemeName): void { themeController.selectTheme(theme); }
function changeLanguage(event: Event): void {
  selectLocale((event.target as HTMLSelectElement).value as AppLocale);
}
</script>

<template>
  <section class="theme-settings" aria-labelledby="theme-settings-title">
    <div class="locale-settings">
      <label for="interface-language">{{ t("settings.language") }}</label>
      <select id="interface-language" :value="locale" @change="changeLanguage">
        <option value="en">English</option>
        <option value="ru">Русский</option>
      </select>
      <p>{{ t("settings.languageHint") }}</p>
      <p v-if="localeStorageError" role="alert" class="theme-error">{{ t("settings.languageStorageError") }}</p>
    </div>
    <h3 id="theme-settings-title">{{ t("theme.title") }}</h3>
    <p>{{ t("theme.description") }}</p>
    <div class="theme-grid" role="group" :aria-label="t('themeLast.themeAria')">
      <button v-for="choice in THEME_CHOICES" :key="choice.key" type="button"
        class="theme-card secondary" :class="{ active: themeState.selected === choice.key }"
        :aria-pressed="themeState.selected === choice.key" @click="select(choice.key)">
        <span class="theme-swatch" :data-choice="choice.key" aria-hidden="true"></span>
        <strong>{{ choice.label }}</strong>
        <small>{{ choice.description }}</small>
      </button>
    </div>
    <p class="theme-current" role="status">{{ t("theme.selected", { name: currentLabel }) }}</p>
    <p v-if="themeState.storageError" role="alert" class="theme-error">{{ themeState.storageError }}</p>
  </section>
</template>

<style scoped>
.theme-settings { display:grid; gap:.75rem; }
.locale-settings { display:grid; gap:.45rem; justify-items:start; }
.locale-settings select { min-width:150px; }
.locale-settings p { margin:0; color:var(--text-secondary); }
.theme-settings > p { color:var(--text-secondary); margin:0; }
.theme-grid { display:grid; grid-template-columns:repeat(auto-fit,minmax(180px,1fr)); gap:.75rem; margin-top:.5rem; }
.theme-card { display:flex; flex-direction:column; align-items:flex-start; gap:.4rem; padding:.8rem; text-align:left;
  background:var(--surface-2); color:var(--text); border:1px solid var(--border); }
.theme-card small { color:var(--text-secondary); line-height:1.35; }
.theme-card.active { border-color:var(--accent); background:var(--surface); }
.theme-card:focus-visible { outline:2px solid var(--focus-ring); outline-offset:2px; }
.theme-swatch { display:block; width:100%; height:2rem; border-radius:.35rem; border:1px solid var(--border); }
.theme-swatch[data-choice="dba"] { background:linear-gradient(100deg,#0d141b 0 44%,#151f29 44% 75%,#38d6c2 75%); }
.theme-swatch[data-choice="ubuntu"] { background:linear-gradient(100deg,#211a29 0 44%,#35283e 44% 75%,#e9a052 75%); }
.theme-swatch[data-choice="light"] { background:linear-gradient(100deg,#f3f6f8 0 44%,#ffffff 44% 75%,#0f9f91 75%); }
.theme-swatch[data-choice="dark"] { background:linear-gradient(100deg,#10151b 0 44%,#1a222c 44% 75%,#82b7ff 75%); }
.theme-current { font-size:.88rem; }
.theme-error { color:var(--error)!important; }
</style>
