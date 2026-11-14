#!/bin/sh
# Renders the macOS app icon (macos/AppIcon.icns, used by the release's OpenKumiteBeast.app) from web/icons/icon.svg,
# which tests/android_icon.rs writes. The drawing is inset as macOS icons are: the rounded square 824 of 1024 pixels,
# centred (viewBox widened from 72 to 72 * 1024 / 824 units). Needs resvg (`cargo install resvg`) and macOS's iconutil.
# The .icns is committed.
set -eu
root="$(cd "$(dirname "$0")/.." && pwd)"
tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT
set="$tmp/AppIcon.iconset"
mkdir -p "$set" "$root/macos"
grep -q 'viewBox="18 18 72 72"' "$root/web/icons/icon.svg" || { echo "web/icons/icon.svg: viewBox is not 18 18 72 72" >&2; exit 1; }
sed 's/viewBox="18 18 72 72"/viewBox="9.2621 9.2621 89.4757 89.4757"/' "$root/web/icons/icon.svg" > "$tmp/icon.svg"
for s in 16 32 128 256 512; do
  resvg -w $s -h $s "$tmp/icon.svg" "$set/icon_${s}x${s}.png"
  resvg -w $((s * 2)) -h $((s * 2)) "$tmp/icon.svg" "$set/icon_${s}x${s}@2x.png"
done
iconutil -c icns -o "$root/macos/AppIcon.icns" "$set"
