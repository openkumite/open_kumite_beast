#!/bin/sh
# Renders the web app's PNG icons (web/icons/) from the SVGs that tests/android_icon.rs writes: run it after
# `OKB_WRITE_ICON=1 cargo test --test android_icon`. Needs resvg (`cargo install resvg`). The PNGs are committed.
set -eu
cd "$(dirname "$0")/../web/icons"
resvg -w 192 -h 192 icon.svg icon-192.png
resvg -w 512 -h 512 icon.svg icon-512.png
resvg -w 512 -h 512 icon-maskable.svg icon-maskable-512.png
# iOS (no transparency; it rounds the corners itself): the maskable drawing's visible middle, 72 of 108 units
sed 's/viewBox="0 0 108 108"/viewBox="18 18 72 72"/' icon-maskable.svg | resvg --resources-dir . -w 180 -h 180 - apple-touch-icon.png
