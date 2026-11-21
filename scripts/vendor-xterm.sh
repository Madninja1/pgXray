#!/usr/bin/env bash
set -euo pipefail

if ! command -v npm >/dev/null 2>&1; then
    echo "npm не найден. Нужен только для однократного получения xterm.js."
    exit 1
fi

if ! command -v sha256sum >/dev/null 2>&1; then
    echo "sha256sum не найден. Нельзя проверить целостность vendor-файлов."
    exit 1
fi

verify_sha256() {
    local expected="$1"
    local file="$2"
    local actual

    actual="$(sha256sum "$file" | awk '{print $1}')"

    if [[ "$actual" != "$expected" ]]; then
        echo "Checksum mismatch: $file" >&2
        echo "expected: $expected" >&2
        echo "actual:   $actual" >&2
        exit 1
    fi
}

root="$(pwd)"

if [[ ! -f "$root/Cargo.toml" || ! -d "$root/ui" ]]; then
    echo "Запусти скрипт из корня pgXray."
    exit 1
fi

tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT

npm install \
    --prefix "$tmp" \
    --no-save \
    @xterm/xterm@6.0.0 \
    @xterm/addon-fit@0.11.0

xterm_js="$tmp/node_modules/@xterm/xterm/lib/xterm.js"
xterm_css="$tmp/node_modules/@xterm/xterm/css/xterm.css"
addon_fit_js="$tmp/node_modules/@xterm/addon-fit/lib/addon-fit.js"

verify_sha256 \
    "14903579ff54664cd72f8e8699e6961a6272c21863ec1c3b118cdc8af5d4a972" \
    "$xterm_js"

verify_sha256 \
    "854a7c0fb70e8b1a083c16797ab827299fb18744f5ad34f227b48337e33293c6" \
    "$xterm_css"

verify_sha256 \
    "ba3ea256ce0620a0992a197d6c9baea64823fc93d8da07a9e366ca9943c18527" \
    "$addon_fit_js"

mkdir -p "$root/ui/vendor/xterm"

cp \
    "$xterm_js" \
    "$root/ui/vendor/xterm/xterm.js"

cp \
    "$xterm_css" \
    "$root/ui/vendor/xterm/xterm.css"

cp \
    "$addon_fit_js" \
    "$root/ui/vendor/xterm/addon-fit.js"

if [[ -f "$tmp/node_modules/@xterm/xterm/LICENSE" ]]; then
    cp \
        "$tmp/node_modules/@xterm/xterm/LICENSE" \
        "$root/ui/vendor/xterm/LICENSE.xterm"
fi

if [[ -f "$tmp/node_modules/@xterm/addon-fit/LICENSE" ]]; then
    cp \
        "$tmp/node_modules/@xterm/addon-fit/LICENSE" \
        "$root/ui/vendor/xterm/LICENSE.addon-fit"
fi

echo "xterm.js установлен в ui/vendor/xterm/"