fn main() {
    // A user-prefix build (no root) lives in ~/.local/lib. The Docker image
    // has libsqlcipher on the default linker path and skips this.
    if let Some(home) = std::env::var_os("HOME") {
        let lib = std::path::Path::new(&home).join(".local/lib");
        if lib.join("libsqlcipher.so").exists() {
            println!("cargo:rustc-link-search=native={}", lib.display());
            println!("cargo:rustc-link-arg=-Wl,-rpath,{}", lib.display());
        }
    }
    println!("cargo:rustc-link-lib=sqlcipher");
}
