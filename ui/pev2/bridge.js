"use strict";

let app = null;

function renderError(text) {
    const root = document.getElementById("app");

    if (app) {
        app.unmount();
        app = null;
    }

    root.replaceChildren();

    const element = document.createElement("div");
    element.className = "pgxray-pev-empty";

    const error = document.createElement("div");
    error.className = "pgxray-pev-error";
    error.textContent = text;

    element.append(error);
    root.append(element);
}

function requirePev2() {
    if (!window.Vue) {
        throw new Error(
            "Vue не загружен. Выполните scripts/vendor-pev2.sh.",
        );
    }

    if (!window.pev2?.Plan) {
        throw new Error(
            "PEV2 не загружен. Выполните scripts/vendor-pev2.sh.",
        );
    }
}

function normalizePlanSource(plan) {
    const source = String(plan ?? "").replace(/^\uFEFF/, "").trim();

    if (!source) {
        throw new Error("Пустой EXPLAIN plan.");
    }
    if (source.length > 32 * 1024 * 1024) {
        throw new Error("EXPLAIN JSON слишком большой для PEV2 (максимум 32 MiB).");
    }

    if (source.startsWith("[") || source.startsWith("{")) {
        let parsed;

        try {
            parsed = JSON.parse(source);
        } catch (error) {
            throw new Error(
                `Анонимизированный EXPLAIN не является JSON: ${error}`,
            );
        }

        if (Array.isArray(parsed) && parsed.length !== 1) {
            throw new Error("PEV2 ожидает ровно один EXPLAIN plan.");
        }
        const root = Array.isArray(parsed)
            ? parsed[0]
            : parsed;

        if (!root || typeof root !== "object" || !root.Plan) {
            throw new Error(
                "JSON не похож на PostgreSQL EXPLAIN FORMAT JSON: отсутствует top-level Plan.",
            );
        }

        if (
            typeof root.Plan !== "object"
            || typeof root.Plan["Node Type"] !== "string"
        ) {
            throw new Error(
                "EXPLAIN JSON повреждён: у корневого Plan отсутствует Node Type.",
            );
        }
    }

    return source;
}

function mountPlan(plan, query = "") {
    requirePev2();

    const source = normalizePlanSource(plan);
    const root = document.getElementById("app");

    if (app) {
        app.unmount();
        app = null;
    }

    root.replaceChildren();

    const { createApp, h } = window.Vue;

    // Use render functions instead of runtime template compilation. The
    // application CSP deliberately does not permit unsafe-eval.
    app = createApp({
        render() {
            return h("div", { class: "pgxray-pev-root" }, [
                h(window.pev2.Plan, {
                    planSource: source,
                    planQuery: String(query ?? ""),
                }),
            ]);
        },
    });
    app.config.errorHandler = (error) => {
        renderError(`PEV2: ${String(error)}`);
        window.parent?.postMessage({ type: "pgxray-pev2-error", error: String(error) }, "*");
    };
    app.mount("#app");
    window.parent?.postMessage({ type: "pgxray-pev2-mounted" }, "*");
}

window.addEventListener("message", (event) => {
    if (event.source !== window.parent) {
        return;
    }

    const data = event.data;

    if (
        !data
        || data.type !== "pgxray-pev2-plan"
    ) {
        return;
    }

    try {
        mountPlan(
            data.plan,
            data.query,
        );
    } catch (error) {
        renderError(
            String(error),
        );
    }
});

window.addEventListener("DOMContentLoaded", () => {
    try {
        requirePev2();
    } catch (error) {
        renderError(
            String(error)
            + "\nИз корня проекта выполните:\n"
            + "./scripts/vendor-pev2.sh",
        );

        return;
    }

    window.parent?.postMessage(
        {
            type: "pgxray-pev2-ready",
        },
        "*",
    );
});