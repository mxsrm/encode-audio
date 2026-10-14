//! Detect the legacy channel-count API, which FFmpeg removed in version 7.

fn main() {
    println!("cargo:rustc-check-cfg=cfg(legacy_channel_count)");
    let library = pkg_config::Config::new()
        .cargo_metadata(false)
        .probe("libavcodec")
        .expect("FFmpeg development libraries and pkg-config are required");
    let major: u32 = library.version.split('.').next().unwrap().parse().unwrap();
    if major < 61 {
        println!("cargo:rustc-cfg=legacy_channel_count");
    }
}
