// build.rs
use std::env;
use std::path::PathBuf;

fn main() {
    let dst = autotools::Config::new(PathBuf::from("third_party/libass"))
        .reconf("-ivf")
        .disable_shared()
        .enable_static()
        .build();

    println!("cargo:rustc-link-search=native={}/lib", dst.display());
    println!("cargo:rustc-link-lib=static=ass");
    pkg_config::Config::new().probe("freetype2").unwrap();
    pkg_config::Config::new().probe("fribidi").unwrap();
    pkg_config::Config::new().probe("harfbuzz").unwrap();
    pkg_config::Config::new().probe("fontconfig").unwrap();

    // 编译我们的 wrapper.c 文件
    cc::Build::new()
        .file("wrapper.c")
        .include(&format!("{}/include", dst.display()))
        .compile("wrapper");

    println!("cargo:rerun-if-changed=wrapper.h");
    println!("cargo:rerun-if-changed=wrapper.c");

    let bindings = bindgen::Builder::default()
        .header("wrapper.h")
        .clang_arg(format!("-I{}/include", dst.display()))
        .parse_callbacks(Box::new(bindgen::CargoCallbacks::new()))
        .generate()
        .expect("无法生成绑定");

    let out_path = PathBuf::from(env::var("OUT_DIR").unwrap());
    bindings
        .write_to_file(out_path.join("bindings.rs"))
        .expect("无法写入绑定文件");
}
