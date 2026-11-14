#!/bin/bash
# Builds the Android APK (docs/android.md): the Rust library with cargo-ndk, the Java activity with javac + d8, the
# package with aapt2, zipalign and apksigner (debug keystore). Downloads nothing.
#
#   android/build.sh                        # arm64-v8a → android/build/okbeast.apk
#   ABIS="arm64-v8a x86_64" android/build.sh
#   SKIP_RUST=1 android/build.sh            # repackage with the libraries of the last build (a Java-only change)
#   RELEASE_KEYSTORE=key.jks RELEASE_KEYSTORE_PASSWORD=… RELEASE_KEY_ALIAS=… VERSION_NAME=0.2.0 VERSION_CODE=200 \
#       ABIS="arm64-v8a x86_64" android/build.sh   # a release: not debuggable, signed with the release key
#   ASSETS=legal android/build.sh           # this folder's files packaged under assets/ (the release's license texts)
#
# Needs: ANDROID_HOME (default ~/Library/Android/sdk) with build-tools and platforms/android-$API, an NDK (the
# newest under $ANDROID_HOME/ndk, or ANDROID_NDK_HOME), cargo-ndk, a JDK (javac, keytool).
set -euo pipefail
cd "$(dirname "$0")/.."
ANDROID_HOME=${ANDROID_HOME:-$HOME/Library/Android/sdk}
API=${API:-35}
MIN_API=26
ABIS=${ABIS:-arm64-v8a}
BT=${BUILD_TOOLS:-$(ls "$ANDROID_HOME/build-tools" | sort -V | tail -1)}
TOOLS=$ANDROID_HOME/build-tools/$BT
JAR=$ANDROID_HOME/platforms/android-$API/android.jar
export ANDROID_NDK_HOME=${ANDROID_NDK_HOME:-$ANDROID_HOME/ndk/$(ls "$ANDROID_HOME/ndk" | sort -V | tail -1)}
OUT=android/build
[ -f "$JAR" ] || { echo "missing $JAR (platforms;android-$API)"; exit 1; }
rm -rf "$OUT/apk" "$OUT/classes" "$OUT/dex"
mkdir -p "$OUT/apk" "$OUT/lib" "$OUT/classes" "$OUT/dex"

# 1. the native library, one per ABI
if [ -z "${SKIP_RUST:-}" ]; then
    rm -rf "$OUT/lib"
    targets=()
    for abi in $ABIS; do targets+=(-t "$abi"); done
    # with the NETWORK menu's room codes (WebRTC through a matchbox signaling server)
    cargo ndk "${targets[@]}" -P "$MIN_API" -o "$OUT/lib" build --release --lib --features netplay-webrtc
fi
cp -R "$OUT/lib" "$OUT/apk/lib"

# 2. the activity
javac --release 11 -nowarn -classpath "$JAR" -d "$OUT/classes" android/java/com/openkumitebeast/game/OkbActivity.java
"$TOOLS/d8" --release --min-api "$MIN_API" --lib "$JAR" --output "$OUT/dex" $(find "$OUT/classes" -name '*.class')

# 3. the package: the manifest (its version overridden by VERSION_NAME / VERSION_CODE when set) and the resources (the
# launcher icon, android/res written by tests/android_icon.rs), the dex, the libraries (stored, page-aligned). Without
# RELEASE_KEYSTORE it is debuggable (`adb shell run-as com.openkumitebeast.game` reaches its files) and signed with the
# debug key; with it, a release: not debuggable, signed with that key (RELEASE_KEYSTORE_PASSWORD for the store and the
# key, RELEASE_KEY_ALIAS). ASSETS, when set, is a folder packaged as the APK's assets/
rm -rf "$OUT/res"
mkdir -p "$OUT/res"
"$TOOLS/aapt2" compile --dir android/res -o "$OUT/res"
link=(--manifest android/AndroidManifest.xml -I "$JAR" -o "$OUT/unsigned.apk")
[ -n "${RELEASE_KEYSTORE:-}" ] || link+=(--debug-mode)
[ -z "${VERSION_NAME:-}" ] || link+=(--replace-version --version-name "$VERSION_NAME")
[ -z "${VERSION_CODE:-}" ] || link+=(--replace-version --version-code "$VERSION_CODE")
[ -z "${ASSETS:-}" ] || link+=(-A "$ASSETS")
"$TOOLS/aapt2" link "${link[@]}" "$OUT"/res/*.flat
cp "$OUT/dex/classes.dex" "$OUT/apk/"
(cd "$OUT/apk" && zip -q ../unsigned.apk classes.dex && zip -q -0 -r ../unsigned.apk lib)
"$TOOLS/zipalign" -f -p 4 "$OUT/unsigned.apk" "$OUT/aligned.apk"
if [ -n "${RELEASE_KEYSTORE:-}" ]; then
    : "${RELEASE_KEYSTORE_PASSWORD:?needed with RELEASE_KEYSTORE}" "${RELEASE_KEY_ALIAS:?needed with RELEASE_KEYSTORE}"
    # the password through the environment, never on the command line
    "$TOOLS/apksigner" sign --ks "$RELEASE_KEYSTORE" --ks-pass env:RELEASE_KEYSTORE_PASSWORD \
        --key-pass env:RELEASE_KEYSTORE_PASSWORD --ks-key-alias "$RELEASE_KEY_ALIAS" --out "$OUT/okbeast.apk" "$OUT/aligned.apk"
else
    KS=${DEBUG_KEYSTORE:-$HOME/.android/debug.keystore}
    if [ ! -f "$KS" ]; then
        KS=$OUT/debug.keystore
        [ -f "$KS" ] || keytool -genkeypair -keystore "$KS" -storepass android -keypass android -alias androiddebugkey \
            -keyalg RSA -keysize 2048 -validity 10000 -dname "CN=Android Debug,O=Android,C=US"
    fi
    "$TOOLS/apksigner" sign --ks "$KS" --ks-pass pass:android --key-pass pass:android --ks-key-alias androiddebugkey \
        --out "$OUT/okbeast.apk" "$OUT/aligned.apk"
fi
rm -f "$OUT/unsigned.apk" "$OUT/aligned.apk" "$OUT/okbeast.apk.idsig"
echo "$OUT/okbeast.apk ($(du -h "$OUT/okbeast.apk" | cut -f1)): adb install -r $OUT/okbeast.apk"
