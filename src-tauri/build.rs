fn main() {
    tauri_build::build();

    // Compile the C USB helper that uses IOKit for UVC extension unit commands
    cc::Build::new()
        .file("src/usb_helper.c")
        .compile("usb_helper");

    // Link IOKit and CoreFoundation frameworks
    println!("cargo:rustc-link-lib=framework=IOKit");
    println!("cargo:rustc-link-lib=framework=CoreFoundation");
}
