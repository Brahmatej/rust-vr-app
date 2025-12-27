#!/bin/bash
set -e

# VR App APK Build Script
# This script packages the native Rust library into an APK

# Configuration
SDK_ROOT="$HOME/Library/Android/sdk"
BUILD_TOOLS="$SDK_ROOT/build-tools/36.0.0"
PLATFORM="$SDK_ROOT/platforms/android-34"
AAPT2="$BUILD_TOOLS/aapt2"
ZIPALIGN="$BUILD_TOOLS/zipalign"
APKSIGNER="$BUILD_TOOLS/apksigner"
PROJECT_DIR="$(cd "$(dirname "$0")" && pwd)"

# Source paths
MANIFEST="$PROJECT_DIR/android/app/src/main/AndroidManifest.xml"
NATIVE_LIB="$PROJECT_DIR/target/aarch64-linux-android/release/libvr_core.so"

# Output paths
OUTPUT_DIR="$PROJECT_DIR/build/apk"
UNSIGNED_APK="$OUTPUT_DIR/vr_app_unsigned.apk"
ALIGNED_APK="$OUTPUT_DIR/vr_app_aligned.apk"
SIGNED_APK="$OUTPUT_DIR/vr_app.apk"
KEYSTORE="$OUTPUT_DIR/debug.keystore"

echo "=== VR App APK Builder ==="

# Create output directory
mkdir -p "$OUTPUT_DIR"
mkdir -p "$OUTPUT_DIR/lib/arm64-v8a"

# Copy native library
echo "Copying native library..."
cp "$NATIVE_LIB" "$OUTPUT_DIR/lib/arm64-v8a/"

# Check if build-tools exists
if [ ! -d "$BUILD_TOOLS" ]; then
    echo "Installing Android build-tools..."
    export JAVA_HOME=/opt/homebrew/opt/openjdk@21/libexec/openjdk.jdk/Contents/Home
    export PATH="$JAVA_HOME/bin:$PATH"
    /opt/homebrew/bin/sdkmanager --sdk_root="$SDK_ROOT" "build-tools;36.0.0" "platforms;android-34"
fi

# Create temporary APK structure
echo "Building APK..."
cd "$OUTPUT_DIR"

# Create minimal APK (just manifest + lib)
echo "Compiling manifest..."
"$AAPT2" link -o "$UNSIGNED_APK" \
    --manifest "$MANIFEST" \
    -I "$PLATFORM/android.jar" \
    --min-sdk-version 24 \
    --target-sdk-version 34 \
    -v

# Add native library to APK
echo "Adding native library to APK..."
cd "$OUTPUT_DIR"
zip -r "$UNSIGNED_APK" lib/

# Align APK
echo "Aligning APK..."
"$ZIPALIGN" -f 4 "$UNSIGNED_APK" "$ALIGNED_APK"

# Create debug keystore if not exists
if [ ! -f "$KEYSTORE" ]; then
    echo "Creating debug keystore..."
    keytool -genkeypair -v \
        -keystore "$KEYSTORE" \
        -alias debug \
        -keyalg RSA \
        -keysize 2048 \
        -validity 10000 \
        -storepass android \
        -keypass android \
        -dname "CN=Debug, OU=Debug, O=Debug, L=Debug, S=Debug, C=US"
fi

# Sign APK
echo "Signing APK..."
"$APKSIGNER" sign \
    --ks "$KEYSTORE" \
    --ks-key-alias debug \
    --ks-pass pass:android \
    --key-pass pass:android \
    --out "$SIGNED_APK" \
    "$ALIGNED_APK"

echo ""
echo "=== BUILD COMPLETE ==="
echo "APK: $SIGNED_APK"
echo ""
echo "To install: ~/Library/Android/sdk/platform-tools/adb install -r $SIGNED_APK"
