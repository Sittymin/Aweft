use std::env;
use std::path::PathBuf;

fn main() {
    // 1. Tell Cargo to rerun this script only if the C source code changes.
    // This is the key for caching the C library compilation.
    println!("cargo:rerun-if-changed=third_party/libass");
    println!("cargo:rerun-if-changed=wrapper.c");
    println!("cargo:rerun-if-changed=wrapper.h");

    // 2. Build the C library using autotools.
    // The .reconf("-ivf") call has been removed to prevent it from modifying
    // source files and causing an unnecessary rebuild loop.
    let dst = autotools::Config::new(PathBuf::from("third_party/libass"))
        .disable_shared()
        .enable_static()
        .build();

    // 3. Compile the C wrapper file.
    cc::Build::new()
        .file("wrapper.c")
        .include(&format!("{}/include", dst.display()))
        .compile("wrapper");

    // 4. Print linker instructions for Cargo.
    println!("cargo:rustc-link-search=native={}/lib", dst.display());
    println!("cargo:rustc-link-lib=static=ass");

    // 5. Probe for other required system libraries.
    // pkg-config will automatically add the necessary linker flags.
    pkg_config::Config::new().probe("freetype2").unwrap();
    pkg_config::Config::new().probe("fribidi").unwrap();
    pkg_config::Config::new().probe("harfbuzz").unwrap();
    pkg_config::Config::new().probe("fontconfig").unwrap();
}
