/// <reference types="vite/client" />
declare module "*.vue" {
  import type { DefineComponent } from "vue";
  const component: DefineComponent<object, object, unknown>;
  export default component;
}

// Tauri CSP allows locally emitted JavaScript assets, not data: scripts.
// Keep the small FitAddon asset external instead of Vite's inline URL.
declare module "*?url&no-inline" {
  const url: string;
  export default url;
}
