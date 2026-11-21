"use strict";

const PGXRAY_THEME_KEY = "pgxray.theme";
const PGXRAY_CUSTOM_THEME_KEY = "pgxray.customTheme";
const PGXRAY_DEFAULT_THEME = "graphite-teal";

const PGXRAY_THEMES = new Set([
    "graphite-teal",
    "forest-mint",
    "indigo-cyan",
    "light",
    "custom",
]);

const CUSTOM_THEME_FIELDS = [
    ["--bg", "Фон приложения", "#0d141b"],
    ["--surface", "Основной блок", "#151f29"],
    ["--surface-2", "Вложенный блок", "#111a23"],
    ["--surface-3", "Глубокий блок / code", "#071016"],

    ["--border", "Граница", "#2b3a46"],
    ["--border-strong", "Сильная граница", "#516472"],
    ["--border-soft", "Мягкая граница", "#364957"],
    ["--border-emphasis", "Акцентная граница", "#395764"],

    ["--text", "Основной текст", "#eef4f8"],
    ["--text-strong", "Сильный текст", "#f5f9fb"],
    ["--text-secondary", "Вторичный текст", "#d8e2e8"],
    ["--text-soft", "Мягкий текст", "#d4dee4"],
    ["--muted", "Muted text", "#9fb0bd"],
    ["--muted-2", "Muted text 2", "#748896"],
    ["--code-text", "Code text", "#e0eaf0"],

    ["--accent", "Акцент", "#38d6c2"],
    ["--accent-contrast", "Текст на акценте", "#062b27"],
    ["--secondary", "Базовый secondary", "#263746"],

    ["--success", "Успех", "#6ee7cf"],
    ["--success-border", "Граница успеха", "#2c6c60"],
    ["--success-bg", "Фон успеха", "#0d211e"],
    ["--message", "Информационный текст", "#a7d8cf"],

    ["--danger", "Базовый danger", "#713440"],
    ["--danger-text", "Текст danger", "#ffe1e5"],
    ["--error", "Ошибка", "#ffb4b8"],
    ["--error-strong", "Сильная ошибка", "#ff8f98"],
    ["--error-focus", "Focus ошибки", "#dc5f6c"],
    ["--error-border", "Граница ошибки", "#7a4a55"],
    ["--error-bg", "Фон ошибки", "#28171c"],

    ["--nav-bg", "Фон навигации", "#151f29"],
    ["--nav-active-bg", "Активная навигация", "#0d141b"],

    ["--button-primary", "Основная кнопка", "#38d6c2"],
    ["--button-primary-text", "Текст основной кнопки", "#062b27"],
    ["--button-secondary", "Вторичная кнопка", "#263746"],
    ["--button-secondary-text", "Текст вторичной кнопки", "#eef4f8"],
    ["--button-danger", "Опасная кнопка", "#713440"],
    ["--button-danger-text", "Текст опасной кнопки", "#ffe1e5"],

    ["--input-bg", "Фон input/select", "#0d141b"],
    ["--input-text", "Текст input/select", "#eef4f8"],

    ["--terminal-bg", "Terminal background", "#0b111b"],
    ["--terminal-text", "Terminal text", "#e6edf5"],
    ["--terminal-cursor", "Terminal cursor", "#70d7c0"],
    ["--terminal-selection", "Terminal selection", "#314157"],
];

function normalizeThemeName(value) {
    return PGXRAY_THEMES.has(value)
        ? value
        : PGXRAY_DEFAULT_THEME;
}

function readStoredThemeName() {
    try {
        return normalizeThemeName(
            localStorage.getItem(PGXRAY_THEME_KEY),
        );
    } catch (_) {
        return PGXRAY_DEFAULT_THEME;
    }
}

function readCustomTheme() {
    try {
        const parsed = JSON.parse(
            localStorage.getItem(PGXRAY_CUSTOM_THEME_KEY) ?? "{}",
        );

        return parsed && typeof parsed === "object"
            ? parsed
            : {};
    } catch (_) {
        return {};
    }
}

function writeCustomTheme(values) {
    try {
        localStorage.setItem(
            PGXRAY_CUSTOM_THEME_KEY,
            JSON.stringify(values),
        );
    } catch (_) {
        // Theme remains active for the current process even if storage is unavailable.
    }
}

function clearInlineThemeVariables() {
    for (const [property] of CUSTOM_THEME_FIELDS) {
        document.documentElement.style.removeProperty(property);
    }
}

function applyCustomThemeVariables(values) {
    clearInlineThemeVariables();

    for (const [property, , fallback] of CUSTOM_THEME_FIELDS) {
        const value = values[property] ?? fallback;

        if (/^#[0-9a-f]{6}$/i.test(value)) {
            document.documentElement.style.setProperty(
                property,
                value,
            );
        }
    }
}

function applyPgXrayTheme(
    theme,
    { persist = true } = {},
) {
    const normalized = normalizeThemeName(theme);

    document.documentElement.dataset.theme = normalized;

    if (normalized === "custom") {
        applyCustomThemeVariables(
            readCustomTheme(),
        );
    } else {
        clearInlineThemeVariables();
    }

    if (persist) {
        try {
            localStorage.setItem(
                PGXRAY_THEME_KEY,
                normalized,
            );
        } catch (_) {
            // Ignore WebView storage errors; current theme is already applied.
        }
    }

    syncTerminalTheme();

    window.dispatchEvent(
        new CustomEvent(
            "pgxray-theme-changed",
            {
                detail: {
                    theme: normalized,
                },
            },
        ),
    );

    return normalized;
}

function terminalThemeFromCss() {
    const root = getComputedStyle(
        document.documentElement,
    );

    const read = (
        name,
        fallback,
    ) => root.getPropertyValue(name).trim() || fallback;

    return {
        background: read("--terminal-bg", "#0b111b"),
        foreground: read("--terminal-text", "#e6edf5"),
        cursor: read("--terminal-cursor", "#70d7c0"),
        selectionBackground: read("--terminal-selection", "#314157"),
    };
}

function syncTerminalTheme() {
    if (
        typeof terminal === "undefined"
        || !terminal
    ) {
        return;
    }

    terminal.options.theme = terminalThemeFromCss();
}

function saveCustomTheme(values) {
    writeCustomTheme(values);
    applyPgXrayTheme("custom");
}

applyPgXrayTheme(
    readStoredThemeName(),
    {
        persist: false,
    },
);