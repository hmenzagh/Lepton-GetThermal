fn main() {
    tauri_build::build();

    // Build libuvc from source
    let mut cmake_config = cmake::Config::new("libuvc");
    cmake_config
        .define("CMAKE_BUILD_TYPE", "Release")
        .define("CMAKE_BUILD_TARGET", "Both")
        .define("CMAKE_POLICY_VERSION_MINIMUM", "3.5");

    // Help cmake find libusb on macOS with Homebrew
    #[cfg(target_os = "macos")]
    {
        if std::path::Path::new("/opt/homebrew").exists() {
            cmake_config.define("CMAKE_PREFIX_PATH", "/opt/homebrew");
        }
    }

    let dst = cmake_config.build();

    // libuvc names the static library "uvcstatic" (libuvcstatic.a)
    println!("cargo:rustc-link-search=native={}/lib", dst.display());
    println!("cargo:rustc-link-search=native={}/build", dst.display());
    println!("cargo:rustc-link-lib=static=uvcstatic");

    // Link libusb (must be installed: brew install libusb)
    // Add Homebrew library path for macOS
    #[cfg(target_os = "macos")]
    {
        if std::path::Path::new("/opt/homebrew/lib").exists() {
            println!("cargo:rustc-link-search=native=/opt/homebrew/lib");
        }
    }
    println!("cargo:rustc-link-lib=usb-1.0");

    // macOS frameworks
    #[cfg(target_os = "macos")]
    {
        println!("cargo:rustc-link-lib=framework=IOKit");
        println!("cargo:rustc-link-lib=framework=CoreFoundation");
        println!("cargo:rustc-link-lib=framework=Security");
    }
}
