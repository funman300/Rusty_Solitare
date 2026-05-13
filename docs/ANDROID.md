# Android build — developer setup

This doc captures the toolchain install + build invocation for the
Android target. Steps are runnable on a fresh Debian 13 (trixie) box;
later sections document what's known to compile, what's stubbed, and
the next milestones.

> **Status (2026-05-07):** First working APK at `fb8b2ac`. 54 MB
> debug-signed `solitaire-quest.apk` for `x86_64-linux-android`. Has
> NOT yet been verified to launch on a device or emulator — that's
> the next milestone.

---

## 1. Toolchain install (Debian 13 / trixie)

Run as one block. Will pull ~15-20 GB of disk between APT, the SDK,
the NDK, the system image, and Rust target sysroots. Requires sudo.

```bash
# 1. JDK 21 (Android tooling needs JDK 17+; Debian 13 default is 21).
sudo apt update && sudo apt install -y openjdk-21-jdk-headless unzip wget

# 2. SDK directory + Google's cmdline-tools bootstrap.
export ANDROID_HOME="$HOME/Android/Sdk"
mkdir -p "$ANDROID_HOME/cmdline-tools"
wget -O /tmp/cmdline-tools.zip \
  https://dl.google.com/android/repository/commandlinetools-linux-11076708_latest.zip
unzip -q /tmp/cmdline-tools.zip -d "$ANDROID_HOME/cmdline-tools"
mv "$ANDROID_HOME/cmdline-tools/cmdline-tools" "$ANDROID_HOME/cmdline-tools/latest"
rm /tmp/cmdline-tools.zip

# 3. Persist env vars.
{
  echo ''
  echo '# Android dev'
  echo 'export ANDROID_HOME="$HOME/Android/Sdk"'
  echo 'export ANDROID_NDK_HOME="$ANDROID_HOME/ndk/26.3.11579264"'
  echo 'export JAVA_HOME="$(dirname $(dirname $(readlink -f $(which java))))"'
  echo 'export PATH="$PATH:$ANDROID_HOME/cmdline-tools/latest/bin:$ANDROID_HOME/platform-tools:$ANDROID_HOME/emulator"'
} >> ~/.bashrc
source ~/.bashrc

# 4. Accept SDK licences (interactive prompts answered by `yes |`).
yes | sdkmanager --licenses

# 5. Platform packages — ~5 GB.
sdkmanager \
  "platform-tools" \
  "platforms;android-34" \
  "build-tools;34.0.0" \
  "ndk;26.3.11579264" \
  "emulator" \
  "system-images;android-34;google_apis;x86_64"

# 6. AVD for testing (one-time).
echo no | avdmanager create avd \
  -n bevy_test \
  -k "system-images;android-34;google_apis;x86_64" \
  -d pixel_7

# 7. Rust cross-compile targets.
rustup target add \
  aarch64-linux-android \
  armv7-linux-androideabi \
  x86_64-linux-android \
  i686-linux-android

# 8. cargo-apk.
cargo install cargo-apk
```

Sanity:

```bash
java --version | head -1            # openjdk 21.0.x
adb --version | head -1             # 35.x or higher
sdkmanager --list_installed | head  # build-tools, emulator, ndk, platforms, system-images
avdmanager list avd | head          # bevy_test
rustup target list --installed | grep android  # 4 targets
cargo apk --help | head -5
```

If `sdkmanager --version` errors with `JAVA_HOME is not set`, the env
section in step 3 didn't apply to your shell — `source ~/.bashrc`
again or open a new terminal.

### Optional: emulator runtime libs

The Android emulator is dynamically linked against X11/GL/audio. If
`emulator -list-avds` works but `emulator -avd bevy_test` complains
about `libX11.so.6`, install:

```bash
sudo apt install -y \
  libx11-6 libxcursor1 libxrandr2 libxi6 libxinerama1 libxxf86vm1 \
  libgl1 libnss3 libpulse0 libxcomposite1
```

Headless emulator launch:

```bash
emulator -avd bevy_test -no-window -gpu swiftshader_indirect &
adb wait-for-device && adb devices
# Stop later:
# adb -s emulator-5554 emu kill
```

Headless + software rendering is fine for "does it boot" smoke tests
but useless for perf measurement — use a physical Pixel-class device
over USB for real numbers.

---

## 2. Build the APK

```bash
cargo apk build -p solitaire_app --target x86_64-linux-android
```

Output:

```
target/debug/apk/solitaire-quest.apk
```

Targets shipped via `[package.metadata.android].build_targets` in
`solitaire_app/Cargo.toml`:

| Target | Use |
|--------|-----|
| `aarch64-linux-android` | Real phones (modern 64-bit ARM) |
| `armv7-linux-androideabi` | Older 32-bit ARM phones |
| `x86_64-linux-android` | The `bevy_test` AVD on this dev box |

Build any of them with `--target <triple>`.

### Known cosmetic warning

After the APK is signed cargo-apk panics with:

```
thread 'main' panicked: Bin is not compatible with Cdylib
```

This happens AFTER the APK is on disk and signed. cargo-apk tries to
also wrap the desktop `[[bin]]` target alongside the `[lib]`. The APK
is valid — the panic is cosmetic. **Always use `--lib`**, which is the
canonical build command (see `CLAUDE.md §15.1`):

```bash
cargo apk build -p solitaire_app --lib
```

Root cause: upstream cargo-apk bug — it does not skip `[[bin]]` targets
when building for Android. No in-repo fix is possible; `--lib` is the
accepted workaround.

---

## 3. Install + run

Physical device:

```bash
adb devices                                    # confirm connection
adb install target/debug/apk/solitaire-quest.apk
adb shell am start -n com.solitairequest.app/android.app.NativeActivity
adb logcat | grep -iE "RustStdoutStderr|solitaire|panic"
```

Emulator:

```bash
emulator -avd bevy_test -no-window -gpu swiftshader_indirect &
adb wait-for-device
adb install target/debug/apk/solitaire-quest.apk
# ... same start + logcat steps as above.
```

If `adb install` errors with `INSTALL_FAILED_NO_MATCHING_ABIS`, the
emulator is x86_64 but the APK was built for arm — rebuild with the
`x86_64-linux-android` target, or add an x86_64 system image to the
AVD.

---

## 4. What's wired vs. what's stubbed

The first build pass (commit `fb8b2ac`) gates four desktop-only
crates / call sites so the workspace cross-compiles. Each gate is
documented at its call site.

| Surface | Desktop | Android |
|---------|---------|---------|
| Bevy windowing | x11 + wayland | `android-native-activity` (NativeActivity glue) |
| Clipboard ("Copy share link") | `arboard` writes URL | Toast surfaces the URL inline |
| OS keychain (JWT tokens) | `keyring` v4 → Secret Service / Keychain / Credential Store | Stub returning `KeychainUnavailable`; sync requires fresh login each launch |
| App entry point | `bin` target → `solitaire_app::run()` | `cdylib` target loaded by NativeActivity |

What's NOT yet ported / not yet measured:

- `dirs::data_dir()` returns `None` on Android. Callers in
  `solitaire_data/src/storage.rs`, `progress.rs`, `replay.rs`,
  `achievements.rs`, `settings.rs` all need an Android-aware
  helper (likely `/data/data/com.solitairequest.app/files`).
- Touch UX pass — hit-target sizes, modal scaling on small screens,
  app lifecycle (suspend / resume), font scaling.
- Android Keystore via JNI for `auth_tokens`.
- JNI ClipboardManager for share links.
- Google Play Games sign-in (the `solitaire_gpgs` crate referenced
  in older docs doesn't yet exist).

---

## 5. Iteration loop

```bash
# Edit code…
cargo build -p solitaire_app                           # desktop sanity
cargo clippy --workspace --all-targets -- -D warnings  # gate
cargo test --workspace                                 # gate
cargo apk build -p solitaire_app --target x86_64-linux-android --lib
adb install -r target/debug/apk/solitaire-quest.apk    # `-r` reinstalls
adb logcat -c && adb shell am start -n com.solitairequest.app/android.app.NativeActivity
adb logcat | grep -iE "RustStdoutStderr|solitaire"
```

`adb logcat` is the canonical way to see Bevy / Rust panic output —
they end up in the `RustStdoutStderr` tag.
