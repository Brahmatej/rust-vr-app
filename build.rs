//! Build script for vr_core
//! Adds necessary linker flags for Android NDK media libraries

fn main() {
    // Only for Android targets
    if std::env::var("CARGO_CFG_TARGET_OS").map_or(false, |os| os == "android") {
        // Link to Android media NDK library for AMediaCodec/AMediaExtractor
        println!("cargo:rustc-link-lib=mediandk");
    }
}
