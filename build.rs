fn link_dir(dir: &str) -> bool {
    let so = std::path::Path::new(dir).join("libsqlcipher.so");
    if !so.exists() {
        return false;
    }
    println!("cargo:rustc-link-search=native={dir}");
    println!("cargo:rustc-link-arg=-Wl,-rpath,{dir}");
    true
}

fn main() {
    // Distro first (Arch sqlcipher is 4.18). /usr/local is where the image
    // installs 4.18. ~/.local is an older build and is used only when neither
    // of those is present.
    let found = link_dir("/usr/lib") || link_dir("/usr/local/lib");
    if !found {
        if let Some(home) = std::env::var_os("HOME") {
            let dir = std::path::Path::new(&home).join(".local/lib");
            let _ = link_dir(&dir.to_string_lossy());
        }
    }
    println!("cargo:rustc-link-lib=sqlcipher");
    println!("cargo:rustc-link-lib=crypto");
}
