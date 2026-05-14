#!/usr/bin/env bash
# Build a self-signed Android APK from solitaire_app's cdylib targets.
#
# Replaces the cargo-apk pipeline with explicit cargo-ndk + aapt2 + apksigner
# steps. The CI runner was hitting an SDK-discovery bug inside cargo-apk's
# ndk-build crate that we couldn't isolate; running each Android toolchain
# step explicitly gives us a debuggable pipeline.
#
# Required environment:
#   ANDROID_HOME            Path to Android SDK root
#   ANDROID_NDK_HOME        Path to the specific NDK version
#   BUILD_TOOLS_VERSION     e.g. "34.0.0"
#   PLATFORM                e.g. "android-34"
#
# Optional environment:
#   PROFILE                 "debug" (default) | "release"
#   APK_OUT                 Output APK path (default: target/$PROFILE/apk/solitaire-quest.apk)
#   KEYSTORE                Path to keystore for signing (default: generates a debug keystore)
#   KEYSTORE_PASS           Keystore password (default: "android" for the generated debug keystore)
#   KEY_ALIAS               Key alias (default: "androiddebugkey")
#   KEY_PASS                Key password (default: same as KEYSTORE_PASS)
#
# Outputs:
#   $APK_OUT                Signed, zipaligned APK
set -euo pipefail

: "${ANDROID_HOME:?ANDROID_HOME must be set}"
: "${ANDROID_NDK_HOME:?ANDROID_NDK_HOME must be set}"
: "${BUILD_TOOLS_VERSION:?BUILD_TOOLS_VERSION must be set}"
: "${PLATFORM:?PLATFORM must be set (e.g. android-34)}"

PROFILE="${PROFILE:-debug}"
APK_OUT="${APK_OUT:-target/${PROFILE}/apk/solitaire-quest.apk}"

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$REPO_ROOT"

BT="$ANDROID_HOME/build-tools/$BUILD_TOOLS_VERSION"
PLATFORM_JAR="$ANDROID_HOME/platforms/$PLATFORM/android.jar"
MANIFEST="solitaire_app/android/AndroidManifest.xml"
RES_DIR="solitaire_app/res"
ASSETS_DIR="assets"

# --- sanity ----------------------------------------------------------------
for f in "$BT/aapt2" "$BT/zipalign" "$BT/apksigner" "$PLATFORM_JAR" "$MANIFEST"; do
  [ -e "$f" ] || { echo "missing: $f"; exit 1; }
done

STAGING="$(mktemp -d)"
trap 'rm -rf "$STAGING"' EXIT
mkdir -p "$STAGING/lib" "$STAGING/compiled-res"

# --- 1. native libraries via cargo-ndk -------------------------------------
# `-o $STAGING/lib` lays out files as $STAGING/lib/<abi>/libsolitaire_app.so
# which is the directory structure the APK expects under lib/.
CARGO_NDK_ARGS=(
  -t arm64-v8a
  -t armeabi-v7a
  -t x86_64
  --platform 26
  -o "$STAGING/lib"
  build --package solitaire_app --lib
)
if [ "$PROFILE" = "release" ]; then
  CARGO_NDK_ARGS+=( --release )
fi
echo ">>> cargo ndk ${CARGO_NDK_ARGS[*]}"
cargo ndk "${CARGO_NDK_ARGS[@]}"

# --- 2. compile + link resources and manifest ------------------------------
if [ -d "$RES_DIR" ]; then
  echo ">>> aapt2 compile resources"
  "$BT/aapt2" compile --dir "$RES_DIR" -o "$STAGING/compiled-res"
fi

LINK_ARGS=(
  link
  -o "$STAGING/app-unsigned.apk"
  -I "$PLATFORM_JAR"
  --manifest "$MANIFEST"
)
[ -d "$ASSETS_DIR" ] && LINK_ARGS+=( -A "$ASSETS_DIR" )
# Add compiled resources if any
shopt -s nullglob
RES_FLATS=( "$STAGING/compiled-res"/*.flat )
shopt -u nullglob
if [ ${#RES_FLATS[@]} -gt 0 ]; then
  LINK_ARGS+=( "${RES_FLATS[@]}" )
fi
echo ">>> aapt2 link"
"$BT/aapt2" "${LINK_ARGS[@]}"

# --- 3. add native libraries to the APK ------------------------------------
echo ">>> bundle native libraries"
( cd "$STAGING" && zip -r -q app-unsigned.apk lib/ )

# --- 4. zipalign -----------------------------------------------------------
echo ">>> zipalign"
"$BT/zipalign" -p -f 4 "$STAGING/app-unsigned.apk" "$STAGING/app-aligned.apk"

# --- 5. sign ---------------------------------------------------------------
if [ -z "${KEYSTORE:-}" ]; then
  # Generate a deterministic debug keystore on the fly.
  KEYSTORE="$STAGING/debug.keystore"
  KEYSTORE_PASS="${KEYSTORE_PASS:-android}"
  KEY_ALIAS="${KEY_ALIAS:-androiddebugkey}"
  KEY_PASS="${KEY_PASS:-$KEYSTORE_PASS}"
  echo ">>> generating debug keystore at $KEYSTORE"
  keytool -genkeypair -v \
    -keystore "$KEYSTORE" \
    -storepass "$KEYSTORE_PASS" \
    -alias "$KEY_ALIAS" \
    -keypass "$KEY_PASS" \
    -keyalg RSA -keysize 2048 -validity 10000 \
    -dname "CN=Android Debug,O=Android,C=US" > /dev/null
fi

KEYSTORE_PASS="${KEYSTORE_PASS:-android}"
KEY_ALIAS="${KEY_ALIAS:-androiddebugkey}"
KEY_PASS="${KEY_PASS:-$KEYSTORE_PASS}"

mkdir -p "$(dirname "$APK_OUT")"
echo ">>> apksigner sign -> $APK_OUT"
"$BT/apksigner" sign \
  --ks "$KEYSTORE" \
  --ks-pass "pass:$KEYSTORE_PASS" \
  --ks-key-alias "$KEY_ALIAS" \
  --key-pass "pass:$KEY_PASS" \
  --out "$APK_OUT" \
  "$STAGING/app-aligned.apk"

echo ">>> verify"
"$BT/apksigner" verify --verbose "$APK_OUT"

echo ">>> done: $APK_OUT"
