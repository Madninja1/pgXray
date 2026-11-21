#!/usr/bin/env bash
set -euo pipefail

# Pinned local dependencies for ui/pev2/.
# The generated application never loads these resources from the network.
PEV2_VERSION="1.25.0"
VUE_VERSION="3.5.28"
BOOTSTRAP_VERSION="5.3.8"

ROOT_DIR="$(
  cd "$(dirname "${BASH_SOURCE[0]}")/.." &&
  pwd
)"

DEST="${ROOT_DIR}/ui/vendor/pev2"

mkdir -p "${DEST}"

download() {
  local url="$1"
  local target="$2"

  printf 'Downloading %s\n' "${url}"

  curl \
    --fail \
    --location \
    --retry 3 \
    --proto '=https' \
    --tlsv1.2 \
    --output "${target}.tmp" \
    "${url}"

  mv \
    "${target}.tmp" \
    "${target}"
}

download \
  "https://unpkg.com/vue@${VUE_VERSION}/dist/vue.global.prod.js" \
  "${DEST}/vue.global.prod.js"

download \
  "https://unpkg.com/bootstrap@${BOOTSTRAP_VERSION}/dist/css/bootstrap.min.css" \
  "${DEST}/bootstrap.min.css"

download \
  "https://unpkg.com/pev2@${PEV2_VERSION}/dist/pev2.umd.js" \
  "${DEST}/pev2.umd.js"

download \
  "https://unpkg.com/pev2@${PEV2_VERSION}/dist/pev2.css" \
  "${DEST}/pev2.css"

download \
  "https://unpkg.com/pev2@${PEV2_VERSION}/LICENSE" \
  "${DEST}/PEV2-LICENSE"

download \
  "https://unpkg.com/vue@${VUE_VERSION}/LICENSE" \
  "${DEST}/VUE-LICENSE"

download \
  "https://unpkg.com/bootstrap@${BOOTSTRAP_VERSION}/LICENSE" \
  "${DEST}/BOOTSTRAP-LICENSE"

cat > "${DEST}/VERSIONS.txt" <<EOF
pev2=${PEV2_VERSION}
vue=${VUE_VERSION}
bootstrap=${BOOTSTRAP_VERSION}
EOF

printf '\nPEV2 vendor assets installed in:\n%s\n' "${DEST}"
printf 'They are now local and can be used without network access.\n'


strip_sourcemap_reference() {
  local file="$1"

  if [[ -f "${file}" ]]; then
    sed -i -E \
      -e 's@/\*# sourceMappingURL=[^*]*\*/@@g' \
      -e 's@//# sourceMappingURL=.*$@@g' \
      "${file}"
  fi
}

strip_sourcemap_reference "${DEST}/vue.global.prod.js"
strip_sourcemap_reference "${DEST}/bootstrap.min.css"
strip_sourcemap_reference "${DEST}/pev2.umd.js"

# Apply the pinned Splitpanes drag/unmount guard after each vendor update.
python3 "${ROOT_DIR}/scripts/patch-pev2-splitpanes.py"
strip_sourcemap_reference "${DEST}/pev2.css"

# Existing xterm vendor files may contain sourceMappingURL comments too.
# They are unrelated to runtime, but WebKit otherwise logs noisy 404/JSON errors.
if [[ -d "${ROOT_DIR}/ui/vendor/xterm" ]]; then
  while IFS= read -r -d '' file; do
    strip_sourcemap_reference "${file}"
  done < <(
    find "${ROOT_DIR}/ui/vendor/xterm" \
      -type f \
      \( -name '*.js' -o -name '*.css' \) \
      -print0
  )
fi

printf 'Source-map references removed from vendored runtime assets.\n'