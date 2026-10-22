//! Shared build script (used by every crate that links fframes with h264: `build = "../../build-support/x264_rpath.rs"`).
//! Link against libx264 ABI 163 when X264_LIB_DIR points at it (the prebuilt FFmpeg that ffmpeg-sys-fframes downloads for
//! aarch64 Linux was built against it; Debian 13 only ships ABI 164/165). The binary finds it at run time through an rpath.
fn main() {
    println!("cargo:rerun-if-env-changed=X264_LIB_DIR");
    if let Ok(dir) = std::env::var("X264_LIB_DIR") {
        if std::path::Path::new(&dir).join("libx264.so.163").exists() {
            println!("cargo:rustc-link-search=native={dir}");
            println!("cargo:rustc-link-arg=-Wl,-rpath,{dir}");
        }
    }
    println!("cargo:rerun-if-changed=../../build-support/x264_rpath.rs");
}
