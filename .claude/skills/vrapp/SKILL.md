---
name: vrapp
description: Build, deploy, debug and modify the VR media player at ~/Downloads/vrapp/vr_core (Rust + wgpu/Vulkan + egui on Android, GeckoView browser, DualSense input). Use for ANY task in this repo - building, installing to the headset, reading logs, or changing renderer/sensor/browser/UI/video code. Covers the non-obvious toolchain (no java_home JDK, no Android Studio), the side-by-side package rule, and the mandatory push rule.
---

# vrapp

Android VR media player: Rust + wgpu (Vulkan) + egui, GeckoView (real Firefox) browser,
PS5 DualSense input, visionOS-style UI. Repo `~/Downloads/vrapp/vr_core`,
remote `Brahmatej/rust-vr-app`, branch `main`.

## Hard rules

1. **Push every commit to `origin main`.** Non-negotiable. A previous local repo was
   lost and months of work went with it. Verify:
   `git ls-remote origin refs/heads/main` must match `git rev-parse HEAD`.
2. **Never touch `com.vrapp.core`.** The dev build installs side-by-side as
   `com.vrapp.core.dev`. The original is the user's working app. Uninstalling the
   *dev* package is a last resort only — it destroys saved browser logins.
3. **Keep the crate at zero compiler warnings.** It is at zero; regressions are not
   acceptable. Silence genuine future-feature code with `#[allow(dead_code)]`
   rather than deleting it.
4. **Install then immediately launch, then push.** Standing user request.
5. Do not use `~/tmp` or `/tmp` for scratch files; use the session scratchpad.

## Build and deploy — use `./deploy.sh`

```
./deploy.sh          # rust + apk + install + launch + crash check + logs
./deploy.sh build    # build only, no device
./deploy.sh logs 200 # just our process's logs
```

Prefer this over running the individual commands. It collapses five tool round
trips into one, and turn count is what makes agent runs expensive.

Raw equivalents, if you must:
```
export PATH="$HOME/.cargo/bin:$PATH" JAVA_HOME=/opt/homebrew/opt/openjdk@21 ANDROID_HOME=$HOME/Library/Android/sdk
cargo ndk -t arm64-v8a -o android/app/src/main/jniLibs build --release
cd android && ~/.gradle/wrapper/dists/gradle-8.13-bin/5xuhj0ry160q40clulazy9h7d/gradle-8.13/bin/gradle --offline assembleDebug
```
There is **no `java_home`-registered JDK and no Android Studio** — the Homebrew
`openjdk@21` path and the cached gradle wrapper binary are required, and gradle
must run `--offline`.

## Reading logs

The device buffer is full of unrelated noise (VidMate, system `NullBinder`).
Always filter to our pid, and drop the per-second spam:
```
./deploy.sh logs 300
```
`HEAD [...]` (sensor thread, ~62/min) and `Overlay HAT` / `D-pad HAT` (every
d-pad edge) will drown everything else otherwise.

**Absence of logs is not evidence of a hang.** Most UI actions log nothing. A
silent render thread usually means focus moved to a surface whose input handler
doesn't log — not a deadlock. (This exact mistake was made once already.)

## Layout

| Path | What |
|---|---|
| `src/lib.rs` | Main loop, `match ui.focus()` input dispatch, transport helpers |
| `src/ui.rs` | egui UI: dock, Media Center, keyboard, tab overview, `Focus`, `Intent` |
| `src/renderer.rs` | wgpu; `Mat4::from_quat(head_orientation.inverse())` builds the view |
| `src/sensors.rs` | Head tracking, ENU→GL basis change, `HEAD_MODE` |
| `src/video_ndk.rs` | AMediaCodec decode; paced by the audio clock |
| `src/webview.rs` | JNI bindings to GeckoView |
| `src/gamepad.rs` | DualSense; sticky `PRESS_LATCH` |
| `src/shaders/main.wgsl` | Screen geometry, dome/180/360/vertical, stereo split |
| `android/.../GeckoViewManager.java` | Gecko sessions, tabs |
| `android/.../MainActivity.java` | JNI surface, audio MediaPlayer. Decompiled-shaped source — match its style |
| `android/.../GamepadOverlay.java` | The ACTUAL key/motion receiver |

## Architecture facts that are easy to get wrong

- **Input arrives via `GamepadOverlay`**, a `WindowManager` overlay view.
  `MainActivity.dispatchKeyEvent` never fires. Do not add Activity-level key handling.
- **`Focus` is the single source of truth** for input routing (`Video`, `Browser`,
  `Dock`, `MediaCenter`, `Keyboard`, `TabOverview`). Per-surface `visible` booleans
  were deliberately deleted. One-shot intents go through `ui::Intent` + a drained
  `VecDeque` — do not reintroduce `*_flag: bool` fields.
- **The UI rasterizes into a fixed 2048×2048 SQUARE texture**; `screen_rect` is
  overridden to 2048×2048 with `set_pixels_per_point(1.0)`. Lay out against that,
  not the device window, or panels land off-centre.
- **Head tracking** needs a two-sided change of basis (`world_fix * q * device_fix`),
  ENU/Z-up → GL/Y-up. No combination of per-axis sign flips can express it, and the
  renderer already inverts once — inverting again makes the world track the head.
- **A/V sync**: audio is a Java `MediaPlayer` and is the MASTER CLOCK
  (`getAudioPositionMs`); the NDK video decoder paces against it. Any transport
  control must drive BOTH pipelines.
- **Dome zoom narrows the arc** (optical zoom); the angular span is set by the
  format (π for 180°, 2π for 360°), not by screen size.
- **GeckoView is UI-thread-affine.** Browser calls must post to the main Looper.

## Device

One phone, ~99% full (~1.7 GB free vs a ~254 MB APK). `INSUFFICIENT_STORAGE` on
install is routine — `deploy.sh` retries after `pm trim-caches`. The screen can
lock mid-session behind a secure keyguard, which suspends the app; **never attempt
to bypass the user's lock** — report that verification was blocked instead.

## Verifying a native change actually shipped

Comparing APK/`.so` hashes fails (the packaged copy is stripped). Instead extract
`lib/arm64-v8a/libvr_core.so` from the APK and grep `strings` for a newly added
log message.
