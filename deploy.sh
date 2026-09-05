#!/usr/bin/env bash
# One-shot build -> APK -> install -> launch -> verify for the VR app.
#
# This exists to collapse what used to be five separate agent tool calls
# (cargo, gradle, install, monkey, logcat) into ONE. Each tool call re-sends the
# whole accumulated context, so turn count - not prompt length - is what makes
# agent runs expensive. Every round trip saved here is saved on every future run.
#
#   ./deploy.sh          build + install + launch + verify
#   ./deploy.sh build    build only (rust + apk), no device needed
#   ./deploy.sh logs     just tail our process's logs
set -uo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PKG=com.vrapp.core.dev
GRADLE=~/.gradle/wrapper/dists/gradle-8.13-bin/5xuhj0ry160q40clulazy9h7d/gradle-8.13/bin/gradle

export PATH="$HOME/.cargo/bin:/opt/homebrew/opt/openjdk@21/bin:$PATH"
export JAVA_HOME=/opt/homebrew/opt/openjdk@21
export ANDROID_HOME="$HOME/Library/Android/sdk"

step() { printf '\n=== %s ===\n' "$1"; }

# Only our process, minus the per-second sensor spam and other apps' noise.
applogs() {
  local pid; pid="$(adb shell pidof $PKG | tr -d '\r')"
  [ -z "$pid" ] && { echo "not running"; return 1; }
  adb logcat -d -t "${1:-300}" 2>/dev/null \
    | grep -E "(^|[^0-9])$pid " \
    | grep -vE "HEAD \[|Overlay HAT|D-pad HAT"
}

if [ "${1:-all}" = logs ]; then applogs "${2:-300}"; exit $?; fi

step "rust (release, arm64)"
cd "$ROOT"
# Build ONCE, keep the output, derive everything else from it. Running cargo
# repeatedly to re-extract warnings would defeat the point of this script.
LOG="$(mktemp)"
cargo ndk -t arm64-v8a -o android/app/src/main/jniLibs build --release >"$LOG" 2>&1
RC=$?
grep -E "^(error|error\[)" "$LOG" | head -30
grep -E "Finished|Copying" "$LOG" || true
if [ $RC -ne 0 ]; then
  echo "RUST BUILD FAILED"; tail -40 "$LOG"; rm -f "$LOG"; exit 1
fi

W=$(grep -c "^warning:" "$LOG")
echo "compiler warnings: $W  (target: 0)"
[ "$W" -gt 0 ] && grep "^warning:" "$LOG" | sort | uniq -c | sort -rn | head -10
rm -f "$LOG"

step "apk"
cd "$ROOT/android"
# Check gradle's OWN exit status, not just whether an APK file exists - a stale
# APK from a previous run will happily sit there after a failed compile and make
# a broken build look successful.
GLOG="$(mktemp)"
"$GRADLE" --offline assembleDebug >"$GLOG" 2>&1
GRC=$?
grep -E "BUILD|error:|FAILED" "$GLOG" | head -20
if [ $GRC -ne 0 ]; then
  echo "APK BUILD FAILED"; grep -E "error:|FAILED" "$GLOG" | head -30; rm -f "$GLOG"; exit 1
fi
rm -f "$GLOG"
APK="$ROOT/android/app/build/outputs/apk/debug/app-debug.apk"
[ -f "$APK" ] || { echo "APK MISSING"; exit 1; }

[ "${1:-all}" = build ] && { echo "build only - done"; exit 0; }

step "install"
cd "$ROOT"
OUT=$(adb install -r "$APK" 2>&1); echo "$OUT"
if echo "$OUT" | grep -q INSUFFICIENT_STORAGE; then
  echo "-> low storage, trimming caches and retrying"
  adb shell pm trim-caches 4G >/dev/null 2>&1
  adb install -r "$APK" 2>&1 | tail -2
fi

step "launch"
adb shell am force-stop $PKG
adb shell monkey -p $PKG -c android.intent.category.LAUNCHER 1 >/dev/null 2>&1
sleep 4
PID="$(adb shell pidof $PKG | tr -d '\r')"
[ -z "$PID" ] && { echo "DID NOT START"; adb logcat -d -t 200 | grep -iE "FATAL|AndroidRuntime" | tail -20; exit 1; }
echo "running, pid $PID"

step "crash check"
adb logcat -d -t 500 2>/dev/null | grep -iE "FATAL|AndroidRuntime|E vr_core" | tail -10 || echo "no crashes"

step "recent app logs"
sleep 2
applogs 60
