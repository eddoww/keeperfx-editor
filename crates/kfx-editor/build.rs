//! Windows: embed the application icon and the version information into the executables.
//!
//! Only when building on Windows for the MSVC target, where `winresource` finds the Windows
//! SDK's resource compiler. Anywhere else, or when the resource compiler is missing, nothing
//! happens (a warning says so), so builds never fail because of it.

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed=../../packaging/icons/kfx-editor.ico");
    let windows = std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows");
    let msvc = std::env::var("CARGO_CFG_TARGET_ENV").as_deref() == Ok("msvc");
    if windows && msvc {
        resources();
    }
}

#[cfg(windows)]
fn resources() {
    let mut r = winresource::WindowsResource::new();
    r.set_icon("../../packaging/icons/kfx-editor.ico")
        .set("ProductName", "KeeperFX Map Editor")
        .set("FileDescription", "KeeperFX Map Editor")
        .set("CompanyName", "KeeperFX Map Editor project")
        .set("LegalCopyright", "GPL-2.0-or-later")
        .set("OriginalFilename", "kfx-editor.exe");
    if let Err(e) = r.compile() {
        println!("cargo:warning=no icon or version information in the executable: {e}");
    }
}

#[cfg(not(windows))]
fn resources() {
    // cross-building for Windows: no resource compiler is assumed
    println!("cargo:warning=no icon or version information: built on a non-Windows host");
}
