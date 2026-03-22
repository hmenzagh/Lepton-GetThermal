// Prevents additional console window on Windows in release, DO NOT REMOVE!!
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    // Redirect stderr to a log file so we can see logs when launched via `open`
    #[cfg(debug_assertions)]
    {
        use std::fs::File;
        use std::os::unix::io::IntoRawFd;
        if let Ok(f) = File::create("/tmp/thermal-v2.log") {
            let fd = f.into_raw_fd();
            unsafe { libc::dup2(fd, 2); }
        }
    }
    thermal_v2_lib::run()
}
