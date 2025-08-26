use std::env;
use std::path::PathBuf;

// 1. Create a custom callback for bindgen.
// This is the key to breaking the dependency loop.
#[derive(Debug)]
struct CustomCallbacks {
    out_path: PathBuf,
}

impl bindgen::callbacks::ParseCallbacks for CustomCallbacks {
    fn include_file(&self, filename: &str) {
        // The default implementation is to emit a `rerun-if-changed` directive for every
        // included file. We override this to prevent it from adding a directive for
        // header files that are generated *within* our OUT_DIR during this build.
        // This breaks the "self-dependency" cycle.
        if !PathBuf::from(filename).starts_with(&self.out_path) {
            println!("cargo:rerun-if-changed={}", filename);
        }
    }
}

fn main() {
    // Tell Cargo to rerun this script only if our direct sources change.
    println!("cargo:rerun-if-changed=third_party/libass");
    println!("cargo:rerun-if-changed=wrapper.h");
    println!("cargo:rerun-if-changed=wrapper.c");

    // Build the C library using autotools.
    // We manually run `autoreconf` in the CI, so it's removed from here.
    let dst = autotools::Config::new(PathBuf::from("third_party/libass"))
        .disable_shared()
        .enable_static()
        .build();

    // Compile our C wrapper file.
    cc::Build::new()
        .file("wrapper.c")
        .include(&format!("{}/include", dst.display()))
        .compile("wrapper");

    // Linker instructions for Cargo.
    println!("cargo:rustc-link-search=native={}/lib", dst.display());
    println!("cargo:rustc-link-lib=static=ass");

    // Probe for other required system libraries.
    pkg_config::Config::new().probe("freetype2").unwrap();
    pkg_config::Config::new().probe("fribidi").unwrap();
    pkg_config::Config::new().probe("harfbuzz").unwrap();
    pkg_config::Config::new().probe("fontconfig").unwrap();

    // 2. Generate bindings on the fly, but with our custom callback.
    let out_path = PathBuf::from(env::var("OUT_DIR").unwrap());
    let bindings = bindgen::Builder::default()
        .header("wrapper.h")
        .clang_arg(format!("-I{}/include", dst.display()))
        // Use our custom callback to prevent the rebuild loop.
        .parse_callbacks(Box::new(CustomCallbacks {
            out_path: out_path.clone(),
        }))
        .generate()
        .expect("Unable to generate bindings");

    // Write the bindings to the $OUT_DIR/bindings.rs file.
    bindings
        .write_to_file(out_path.join("bindings.rs"))
        .expect("Couldn't write bindings!");
}
