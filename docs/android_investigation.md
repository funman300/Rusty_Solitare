# Android Port Investigation

> **Date:** 2026-04-28  
> **Author:** Claude Code  
> **Scope:** Feasibility analysis for porting Solitaire Quest to Android using cargo-mobile2

---

## Summary

A working Android port is feasible but not trivial. The core game logic (`solitaire_core`, `solitaire_sync`) compiles to Android without changes. Every other crate requires at least minor surgery. The biggest blockers are the `keyring` crate (no Android backend), the `kira`/`AudioManager` audio stack (`DefaultBackend` uses CPAL which targets desktop), and the `dirs` crate returning `None` on Android in its current usage. Touch input already has a solid foundation in `input_plugin.rs`. Estimated effort from a clean Android toolchain is **12–18 developer-days** to reach a playable-but-rough state.

---

## 1. Bevy on Android — Current Status

Bevy's Android support is community-maintained via the `winit` backend and is usable but carries known rough edges as of the 0.15/0.16 generation.

**What works:**
- Basic rendering via Vulkan (through `wgpu`). OpenGL ES fallback is available for older devices.
- Touch input events: Bevy's `TouchInput` events and the `Touches` resource are populated from Android `MotionEvent`s via `winit`. The existing `touch_start_drag`, `touch_follow_drag`, `touch_end_drag`, and `handle_touch_stock_tap` systems in `input_plugin.rs` will function correctly — this was already written with multi-touch in mind and uses `TouchPhase::Started/Moved/Ended/Canceled` cleanly.
- Bevy UI (the `bevy::ui` module used for all overlays).
- `WindowResized` events fire correctly, so the layout system will recompute for any screen size.

**What does not work / needs attention:**
- **`bevy/dynamic_linking`**: The dynamic linking feature must be stripped from any Android build profile. Dynamic linking is a desktop-only development shortcut; Android requires static linking.
- **Fixed window size**: `main.rs` sets `resolution: (1280u32, 800u32)`. On Android the window is always the full display. This value is harmlessly overridden by the OS, but `min_width`/`min_height` constraints should be removed or set to 0 for Android to avoid Winit warnings.
- **`F11` fullscreen toggle** (`handle_fullscreen` in `input_plugin.rs`): `WindowMode::BorderlessFullscreen` is desktop-only. On Android it should be a no-op.
- **Keyboard shortcuts**: The entire `handle_keyboard_core`, `handle_keyboard_hint`, `handle_keyboard_forfeit` systems are desktop-only workflows. They will not crash, but they are dead code on Android. No touchscreen replacement for Undo (U), New Game (N), Draw (D/Space), Hint (H), Forfeit (G) exists yet — these need an on-screen UI.
- **`CursorPlugin`**: The custom cursor sprite plugin is irrelevant on Android (no cursor). Harmless to leave registered, but it uses `PrimaryWindow` cursor APIs that may panic or warn on Android.

**cargo-mobile2 integration for Bevy:**
The standard path is:
1. Install `cargo-mobile2`: `cargo install --locked cargo-mobile2`
2. Run `cargo mobile init` in the workspace root. This generates an `android/` directory with the Gradle project, `AndroidManifest.xml`, and JNI glue.
3. cargo-mobile2 targets the `solitaire_app` binary crate (the thin entry point). The generated `lib.rs` shim calls `android_main` via `bevy::winit`'s Android entry point.
4. The `solitaire_app` crate needs a `[lib]` target added alongside the existing `[[bin]]`, with `crate-type = ["cdylib"]`, used only when building for Android.

**Required `Cargo.toml` changes (workspace level):**
```toml
[target.'cfg(target_os = "android")'.dependencies]
# android_logger and ndk-glue wiring are handled by cargo-mobile2's generated shim.
# No direct ndk-glue dependency is needed in app code when using Bevy + cargo-mobile2.
```

**NDK version:** Android NDK r25c or r26 LTS is the tested range for `wgpu`/Vulkan on Android. NDK r27+ may work but has had compatibility reports with CPAL. Set `ANDROID_NDK_ROOT` to the NDK root; the minimum API level should be 26 (Android 8.0) for Vulkan stability.

---

## 2. Audio — `kira` + `DefaultBackend`

**The problem:**  
`solitaire_engine/src/audio_plugin.rs` creates an `AudioManager<DefaultBackend>`. `kira`'s `DefaultBackend` is an alias for `CpalBackend`, which wraps CPAL. CPAL's Android backend uses OpenSL ES and is functional but historically fragile. As of kira 0.9+, `kira` no longer bundles its own CPAL backend by default in the same way — the `DefaultBackend` feature must be enabled explicitly and requires `cpal` with the Android feature.

**Current code behavior:**  
The `AudioPlugin::build` already handles the "no audio device" case gracefully:
```rust
let mut manager = AudioManager::<DefaultBackend>::new(AudioManagerSettings::default()).ok();
if manager.is_none() {
    warn!("audio device unavailable; SFX disabled");
}
```
This means if the audio manager fails to initialise on Android, the game continues silently. This is acceptable as a first-pass fallback.

**What is needed for working audio on Android:**
- Add `kira` dependency with `cpal` backend enabled for Android: The `kira` workspace dependency currently specifies `version = "0.12"`. Verify that `kira/Cargo.toml` exposes a `cpal` feature (or that `DefaultBackend` compiles on Android targets with NDK). If not, a `CpalBackend` with `cpal = { features = ["oboe"] }` may be needed.
- The `NonSend` resource `AudioState` should compile fine — `NonSend` is legal in Bevy Android builds.
- `include_bytes!` for the WAV assets is compile-time and unaffected by platform.

**Recommendation:** Defer full audio verification to a device test. The graceful fallback means a silent-but-working first build is achievable without resolving this.

---

## 3. `keyring` Crate — No Android Backend

**The problem:**  
`keyring = "2"` is used in `solitaire_data/src/auth_tokens.rs` to store JWT access and refresh tokens in the OS keychain. The `keyring` crate's Android backend does not exist — as of v2.x, supported backends are: macOS Keychain, Windows Credential Manager, Linux Secret Service (D-Bus), and iOS Keychain. There is no Android KeyStore backend.

On Android, `Entry::new(...)` will return `keyring::Error::NoStorageAccess`, which the existing code already maps to `TokenError::KeychainUnavailable`. So the code will not crash — it will simply fail every token store/load operation.

**Current failure mode:**  
Every call to `store_tokens`, `load_access_token`, `load_refresh_token`, or `delete_tokens` will return `Err(TokenError::KeychainUnavailable(...))`. The sync client in `sync_client.rs` needs to be verified to handle this gracefully rather than propagating an error that disables sync entirely.

**Options for Android credential storage:**

| Option | Security | Effort | Notes |
|---|---|---|---|
| **In-memory only (prompt re-login each session)** | N/A | 1 day | Simplest. On `TokenError::KeychainUnavailable`, the `SyncProvider` returns `SyncError::Auth`, user is prompted to log in. Already architecturally supported. |
| **Encrypted `SharedPreferences` equivalent via JNI** | Good | 4–6 days | Call Android's `EncryptedSharedPreferences` (Jetpack Security) via JNI. Significant JNI boilerplate. |
| **AES-256 file encryption using Android Keystore via JNI** | Excellent | 5–8 days | Proper Android keychain equivalent. Complex JNI. |
| **Store in app-private file, unencrypted** | Poor | 0.5 days | Only acceptable during development. Never ship. |

**Recommended approach (first pass):** Use the in-memory / re-login-each-session path. The existing `TokenError::KeychainUnavailable` variant already exists for exactly this reason (Linux without a running secret service). The `SyncPlugin` should detect this on startup and present a "Sync unavailable — please log in" message rather than a hard error. This requires:
1. Conditional compilation: when `cfg(target_os = "android")`, replace the `keyring` calls with a no-op in-memory store (a simple `Mutex<HashMap<String, String>>`).
2. A `#[cfg(not(target_os = "android"))]` guard on the `keyring` import/dependency in `solitaire_data/Cargo.toml`.

**Required `solitaire_data/Cargo.toml` change:**
```toml
[target.'cfg(not(target_os = "android"))'.dependencies]
keyring = { workspace = true }

[target.'cfg(target_os = "android")'.dependencies]
# keyring is replaced by in-memory storage; no dependency needed
```

---

## 4. `dirs` Crate — Data Directory on Android

**The problem:**  
`storage.rs` and other persistence modules use `dirs::data_dir()` to locate `~/.local/share/solitaire_quest/` (or platform equivalent). On Android, `dirs::data_dir()` returns `None` because there is no `XDG_DATA_HOME` and the `dirs` crate does not implement an Android-specific path.

**Current code behavior:**  
All persistence functions already handle `None` gracefully (returning default values or `Err`), consistent with the CLAUDE.md lesson about `dirs::data_dir()`. Stats and progress will silently not persist across sessions if `data_dir()` returns `None`.

**Fix required:**  
Android apps should store private data in the app's internal storage directory, obtained via JNI: `context.getFilesDir()`. This requires either:
- A thin JNI helper (via `jni` crate) called once on startup to obtain the path and store it as a global.
- Or passing the path in via the `android_main` entry point using `cargo-mobile2`'s `AndroidApp` handle, which exposes `internal_data_path()`.

The `cargo-mobile2` + Bevy path exposes an `AndroidApp` via `bevy::winit`'s Android entry point. Bevy 0.13+ passes `AndroidApp` through `WinitPlugin`, and it is accessible via a Bevy resource. A startup system can extract `app.internal_data_path()` and insert a `PlatformDataDirResource` that the storage functions read instead of calling `dirs::data_dir()`.

**Effort:** 1–2 days to implement the override and thread it through all `storage.rs` / `progress.rs` / `settings.rs` / `achievements.rs` call sites.

---

## 5. Touch Input — Current State and Gaps

**What already exists (strong foundation):**

The `InputPlugin` in `input_plugin.rs` has a complete parallel touch pipeline:

| System | Purpose | Status |
|---|---|---|
| `handle_touch_stock_tap` | Tap the stock pile to draw | Complete |
| `touch_start_drag` | Begin a touch drag on a face-up card | Complete |
| `touch_follow_drag` | Move card(s) with the active finger | Complete |
| `touch_end_drag` | Resolve the drag (move or reject) | Complete |

The touch systems use `TouchInput` events and the `Touches` resource, map touch IDs to `DragState.active_touch_id` to prevent multi-finger conflicts, and share the same `DragState`, `MoveRequestEvent`, `MoveRejectedEvent`, and `StateChangedEvent` infrastructure as the mouse pipeline. The drag threshold (`tuning.drag_threshold_px`) applies identically.

**Gaps for a production Android experience:**

1. **No double-tap equivalent for auto-move**: `handle_double_click` is mouse-only. Android users need a double-tap to trigger the same "move to best destination" logic. The `handle_double_click` system checks `buttons.just_pressed(MouseButton::Left)` and will be inert on Android. Estimated: 1 day.

2. **No touch equivalent for keyboard actions**: Undo, New Game, Draw (when stock is visible but tapping it is awkward), Hint, and Forfeit have no on-screen buttons. These need an Android-specific UI bar or gesture (e.g. two-finger tap for undo). Estimated: 2–3 days for a minimal floating action button strip.

3. **Drag threshold tuning**: The threshold is in `AnimationTuning` (`tuning.drag_threshold_px`). Touch screens typically need a larger threshold than mouse (physical screens have more accidental movement during a tap). The current value should be evaluated on a real device and likely increased for touch.

4. **No long-press for right-click equivalent**: The right-click highlight/hint glow (`HintHighlightTimer`) is triggered via right mouse button. Long-press detection is not yet implemented. This is a missing feature but not a blocker for basic play.

5. **`handle_double_click` uses `LocalDateTime`-based timing via `Time`**: This will work on Android, but `DOUBLE_CLICK_WINDOW = 0.35s` may feel too tight on touch. Should be configurable.

---

## 6. Additional Issues Not in Scope of the Four Research Areas

**`CursorPlugin`:** Uses Bevy's cursor APIs which are desktop-only. Should be conditionally compiled out on Android with `#[cfg(not(target_os = "android"))]`.

**`reqwest` with `rustls-native-certs`:** The `reqwest` dependency uses `rustls` with native root certificates. On Android, `rustls-native-certs` reads system certificates differently (via the `android_system_properties` crate internally). This generally works but should be tested; Android's certificate store is in a non-standard location vs Linux.

**App lifecycle (suspend/resume):** Android can suspend the process at any time. Bevy handles `WindowEvent::Suspended` and `WindowEvent::Resumed` via `winit`, pausing the render loop. The `SyncPlugin`'s "push on exit" path (`AppExit` event) should also trigger on `WindowEvent::Suspended` to avoid data loss when the user backgrounds the app. This is a separate feature (1 day).

**No `sqlx` on Android:** `solitaire_server` is a server binary and is never built for Android. The `sqlx` dependency only exists in `solitaire_server/Cargo.toml` and will not affect Android builds of the client crates.

**`solitaire_assetgen`:** The asset generation tool is desktop-only and not part of the client build. Unaffected.

---

## 7. Required Changes Per Crate

### `solitaire_core` and `solitaire_sync`
No changes required. Both are pure Rust with no platform dependencies.

### `solitaire_data`
| Change | Effort |
|---|---|
| Gate `keyring` dependency on `#[cfg(not(target_os = "android"))]` | 0.5 days |
| Implement `auth_tokens.rs` in-memory fallback for Android | 1 day |
| Add `internal_data_path()` override for `dirs::data_dir()` on Android | 1.5 days |
| Audit all `dirs::data_dir()` / `settings_file_path()` call sites to accept injected path | 0.5 days |

### `solitaire_engine`
| Change | Effort |
|---|---|
| Conditionally disable `CursorPlugin` on Android | 0.5 days |
| Disable `handle_fullscreen` on Android (or make it a no-op) | 0.25 days |
| Implement double-tap for auto-move (touch equivalent of `handle_double_click`) | 1 day |
| On-screen action bar for Undo, New Game, Hint (minimal floating buttons) | 2.5 days |
| Tune drag threshold for touch; expose as a platform-specific tuning constant | 0.5 days |
| Trigger sync push on `WindowEvent::Suspended` in `SyncPlugin` | 1 day |
| Verify `kira` audio on Android (test `DefaultBackend` / CPAL; implement fallback if needed) | 1–2 days |

### `solitaire_app`
| Change | Effort |
|---|---|
| Add `[lib]` target with `crate-type = ["cdylib"]` for Android builds | 0.25 days |
| Create `src/lib.rs` (or `src/android.rs`) Android entry point calling `android_main` | 0.5 days |
| Remove or guard fixed `resolution` / `resize_constraints` for Android | 0.25 days |
| Pass `AndroidApp::internal_data_path()` to a startup resource | 0.5 days |

### Build / Toolchain
| Change | Effort |
|---|---|
| Install cargo-mobile2, Android NDK r25c/r26, `aarch64-linux-android` target | 1 day |
| Run `cargo mobile init`, configure `android/` Gradle project | 0.5 days |
| Get a first build compiling (resolve linker / NDK issues) | 1–2 days |

---

## 8. Estimated Effort

| Phase | Description | Days |
|---|---|---|
| Toolchain setup | NDK, cargo-mobile2, first compile | 2–3 |
| `solitaire_data` Android adaptations | keyring fallback, data dir | 3 |
| `solitaire_app` Android entry point | cdylib, AndroidApp wiring | 1 |
| `solitaire_engine` guards and fixes | cursor, fullscreen, audio verify | 2–3 |
| Touch UX improvements | double-tap, action bar, threshold tuning | 4–5 |
| Testing on real device / emulator | iteration, lifecycle edge cases | 2–3 |
| **Total** | | **14–17 days** |

This produces a playable, functionally complete Android build. It does not include Play Store preparation (signing keys, metadata, icon set, permissions manifest tuning) which would add 1–2 more days.

---

## 9. Recommended First Step

**Get the workspace to compile for `aarch64-linux-android` without running.**

This surfaces all the real linker and dependency errors before writing any gameplay code:

```bash
# Install toolchain
rustup target add aarch64-linux-android
cargo install --locked cargo-mobile2

# In the workspace root:
cargo mobile init   # generates android/ directory

# Attempt a library build targeting Android
cargo build -p solitaire_app --target aarch64-linux-android 2>&1 | head -60
```

The first build will fail on `keyring` (no Android backend) and likely on `dirs`. Fixing those two in `solitaire_data` — gate `keyring` behind `cfg(not(target_os = "android"))` and stub the data directory — will probably get the workspace to a clean compile. From there, the path to a running APK is incremental.

Do not attempt to resolve audio or touch UX until the build compiles cleanly. Compile errors are the only true blockers; the rest are feature gaps.
