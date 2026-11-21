import { defineConfig, type Plugin } from "vite";
import vue from "@vitejs/plugin-vue";
import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { dirname, resolve } from "node:path";

// Reuse already-vendored PEV2 without loading its Vue runtime in our main Vue app.
// Serve identical isolated resources in Vite dev and in the packaged Vue build.
const vueRoot = dirname(fileURLToPath(import.meta.url));
const root = resolve(vueRoot, "../ui");
const pev2Files = [
  "pev2/index.html", "pev2/bridge.js",
  "vendor/pev2/bootstrap.min.css", "vendor/pev2/pev2.css",
  "vendor/pev2/vue.global.prod.js", "vendor/pev2/pev2.umd.js",
  "vendor/pev2/BOOTSTRAP-LICENSE", "vendor/pev2/PEV2-LICENSE",
  "vendor/pev2/VUE-LICENSE", "vendor/pev2/VERSIONS.txt",
];
function embeddedPev2(): Plugin {
  return {
    name: "pgxray-embedded-pev2",
    configureServer(server) {
      server.middlewares.use((request, response, next) => {
        const path = (request.url ?? "").split("?")[0].replace(/^\//, "");
        if (!pev2Files.includes(path)) return next();
        const type = path.endsWith(".html") ? "text/html; charset=utf-8"
          : path.endsWith(".js") ? "text/javascript; charset=utf-8"
          : path.endsWith(".css") ? "text/css; charset=utf-8" : "text/plain; charset=utf-8";
        response.setHeader("Content-Type", type);
        response.end(readFileSync(resolve(root, path)));
      });
    },
    generateBundle() {
      for (const path of pev2Files) {
        this.emitFile({ type: "asset", fileName: path, source: readFileSync(resolve(root, path)) });
      }
    },
  };
}

export default defineConfig({
  plugins: [vue(), embeddedPev2()],
  server: {
    host: "127.0.0.1", port: 5173, strictPort: true,
    // The Vue entry point imports shared theme tokens and vendored xterm assets
    // from ../ui. Explicitly allow only those directories instead of the whole repo.
    fs: { allow: [vueRoot, resolve(root, "styles"), resolve(root, "vendor/xterm")] },
  },
  preview: { host: "127.0.0.1" },
  build: { outDir: "build" }, // Keep isolated from tsc's dist/ test output.
});
