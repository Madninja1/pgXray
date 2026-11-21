<script setup lang="ts">
type IconName =
  | "database" | "plug" | "activity" | "chart" | "history" | "compare"
  | "tools" | "terminal" | "settings" | "users" | "palette" | "scroll"
  | "folder" | "shield" | "refresh" | "plus" | "ellipsis" | "pin"
  | "pencil" | "key" | "trash" | "x" | "eye" | "eye-off" | "logout"
  | "chevron-left" | "chevron-right";

const props = defineProps<{ name: IconName; size?: number }>();
// Local icon paths avoid a new runtime dependency. All are decorative; the
// surrounding control provides its accessible name.
const paths: Record<IconName, string[]> = {
  database: ["M12 3C7.6 3 4 4.8 4 7s3.6 4 8 4 8-1.8 8-4-3.6-4-8-4Z", "M4 7v5c0 2.2 3.6 4 8 4s8-1.8 8-4V7", "M4 12v5c0 2.2 3.6 4 8 4s8-1.8 8-4v-5"],
  plug: ["M12 22v-6", "M7 8V3", "M17 8V3", "M5 8h14v3a7 7 0 0 1-14 0V8Z"],
  activity: ["M3 12h4l3-7 4 14 3-7h4"],
  chart: ["M4 20V12", "M10 20V5", "M16 20v-9", "M22 20H2"],
  history: ["M3 12a9 9 0 1 0 3-6.7", "M3 4v6h6", "M12 7v5l3 2"],
  compare: ["M8 3H4v4", "M4 3l7 7", "M16 21h4v-4", "M20 21l-7-7", "M13 4h7v7", "M11 20H4v-7"],
  tools: ["M14.7 6.3a5 5 0 0 0-6.4 6.4L3 18l3 3 5.3-5.3a5 5 0 0 0 6.4-6.4", "M16 3l5 5"],
  terminal: ["M4 5h16v14H4z", "m8 9 3 3-3 3", "M13 16h4"],
  settings: ["M12 3v3", "M12 18v3", "M3 12h3", "M18 12h3", "m5.6 5.6 2.1 2.1", "m16.3 16.3 2.1 2.1", "m5.6 18.4 2.1-2.1", "m16.3 7.7 2.1-2.1", "M12 8a4 4 0 1 0 0 8 4 4 0 0 0 0-8Z"],
  users: ["M16 21v-2a4 4 0 0 0-4-4H6a4 4 0 0 0-4 4v2", "M9 11a4 4 0 1 0 0-8 4 4 0 0 0 0 8Z", "M22 21v-2a4 4 0 0 0-3-3.9", "M16 3.1a4 4 0 0 1 0 7.8"],
  palette: ["M12 3a9 9 0 1 0 0 18h1a2 2 0 0 0 2-2c0-1-.5-1.3-.5-2a2 2 0 0 1 2-2H19a2 2 0 0 0 2-2 10 10 0 0 0-9-10Z", "M7 11h.01", "M10 7h.01", "M15 7h.01", "M18 11h.01"],
  scroll: ["M6 3h12v15a3 3 0 0 1-3 3H5a3 3 0 0 1-3-3v-2h4", "M6 16v2a3 3 0 0 1-3 3", "M10 8h5", "M10 12h5"],
  folder: ["M3 6h7l2 2h9v12H3V6Z"],
  pin: ["m16 3 5 5", "m10 9 7-4 3 3-4 7", "m4 20 7-7", "m9 10 5 5"],
  shield: ["M12 2 20 6v6c0 5-3 8-8 10-5-2-8-5-8-10V6l8-4Z", "m9 12 2 2 4-4"],
  refresh: ["M20 11a8 8 0 0 0-14-5L3 9", "M3 4v5h5", "M4 13a8 8 0 0 0 14 5l3-3", "M16 15h5v5"],
  plus: ["M12 5v14", "M5 12h14"],
  ellipsis: ["M4 12h.01", "M12 12h.01", "M20 12h.01"],
  pencil: ["m4 20 4.5-1 11-11a2 2 0 0 0-3-3l-11 11L4 20Z", "m14 6 3 3"],
  key: ["M8 14a5 5 0 1 0-5-5 5 5 0 0 0 5 5Z", "m12 10 11 11", "M18 18l3-3", "M16 16l2-2"],
  trash: ["M3 6h18", "M8 6V4h8v2", "M5 6l1 15h12l1-15", "M10 10v7", "M14 10v7"],
  x: ["M5 5 19 19", "M19 5 5 19"],
  eye: ["M2 12s3.5-7 10-7 10 7 10 7-3.5 7-10 7S2 12 2 12Z", "M12 9a3 3 0 1 0 0 6 3 3 0 0 0 0-6Z"],
  "eye-off": ["M3 3l18 18", "M10 5.2A12 12 0 0 1 12 5c6.5 0 10 7 10 7a13 13 0 0 1-3.2 4.1", "M6 6.5C3.5 8.4 2 12 2 12s3.5 7 10 7c1.6 0 3-.4 4.3-1", "M10.4 10.4a2.3 2.3 0 0 0 3.2 3.2"],
  "chevron-left": ["m15 18-6-6 6-6"],
  "chevron-right": ["m9 18 6-6-6-6"],
  logout: ["M9 21H5a2 2 0 0 1-2-2V5a2 2 0 0 1 2-2h4", "m16 17 5-5-5-5", "M21 12H9"],
};
</script>

<template>
  <svg xmlns="http://www.w3.org/2000/svg" :width="props.size ?? 18" :height="props.size ?? 18"
    viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.7"
    stroke-linecap="round" stroke-linejoin="round" aria-hidden="true" focusable="false">
    <path v-for="(path, i) in paths[props.name]" :key="i" :d="path" />
  </svg>
</template>
