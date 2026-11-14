#!/bin/sh
# Trunk post_build hook (Trunk.toml): writes the web app's service worker into the build. A release build gets
# web/sw.js with VERSION (a hash of every file) and FILES (every file of the build, "./" for index.html); a debug build
# (`trunk serve`) gets one that clears the okbeast-* caches and unregisters itself, so development never sees a cached
# build. Fails on a file name outside [A-Za-z0-9._/-] (it would break the list and silently disable offline support).
set -eu
dir=${TRUNK_STAGING_DIR:?run by trunk}
if [ "${TRUNK_PROFILE:-release}" != release ]; then
    cat > "$dir/sw.js" <<'JS'
// debug build: no caching; remove an earlier release build's worker and caches
self.addEventListener("install", () => self.skipWaiting());
self.addEventListener("activate", (e) =>
  e.waitUntil(
    caches
      .keys()
      .then((k) => Promise.all(k.filter((n) => n.startsWith("okbeast-")).map((n) => caches.delete(n))))
      .then(() => self.registration.unregister()),
  ),
);
JS
    exit 0
fi
cd "$dir"
files=$(find . -type f ! -name sw.js | sed 's|^\./||' | LC_ALL=C sort)
bad=$(printf '%s\n' "$files" | LC_ALL=C grep -v '^[A-Za-z0-9._/-]*$' || true)
[ -z "$bad" ] || { echo "service worker: unsupported characters in file name(s): $bad" >&2; exit 1; }
version=$(for f in $files; do printf '%s  %s\n' "$(cksum < "$f" | cut -d' ' -f1-2)" "$f"; done | cksum | cut -d' ' -f1)
list=$(for f in $files; do [ "$f" = index.html ] && f=; printf '"./%s",' "$f"; done)
sed -e "s|__VERSION__|$version|" -e "s|__FILES__|[${list%,}]|" sw.js > sw.js.tmp && mv sw.js.tmp sw.js
echo "service worker: version $version, $(echo "$files" | wc -l | tr -d ' ') files"
