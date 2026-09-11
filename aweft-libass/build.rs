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

    let target = env::var("TARGET").unwrap_or_default();
    let is_android = target.contains("android");

    // Build the C library using autotools.
    // We manually run `autoreconf` in the CI, so it's removed from here.
    let mut config = autotools::Config::new(PathBuf::from("third_party/libass"));
    config.disable_shared().enable_static();
    // Android：交叉三元组 + 关 fontconfig。autotools 包的 target() 会转成
    // configure 的 --host；CC 等环境由 flake 的 shellHook 指向 NDK 包装。
    // 四个依赖里 freetype/fribidi/harfbuzz 由 android-deps.sh 预编好
    // （TARGET_PKG_CONFIG_PATH 指向其 .pc），fontconfig 在 Android 上没有
    // 系统生态，直接禁用——字体走内嵌（ass_add_font）与系统目录。
    if is_android {
        // xbuild 给交叉编译的环境变量是横线式（CC_aarch64-linux-android=clang，
        // xbuild/src/cargo/mod.rs:416 cc_triple_env），而 cc-rs 查变量的优先级把
        // 横线式排在下划线式之前（cc-1.4.0/src/lib.rs:4213：CC_<triple> →
        // CC_<triple 下划线> → TARGET_CC → CC），flake 导出的那份就被盖掉了。
        // autotools 把 cc-rs 选出的编译器原样交给 configure
        // （autotools-0.2.7/src/lib.rs:605），于是 configure 用上 nixpkgs 的 clang，
        // 它的 cc-wrapper 没有 android 的 libclang_rt.builtins.a，
        // 报“C compiler cannot create executables”（exit 77）。
        // 删掉横线式三项即回落到 flake 的 NDK 包装；CFLAGS 保留，
        // 让 C 侧和 Rust 侧链到同一个 sysroot 与 API 级别。
        for tool in ["CC", "CXX", "AR"] {
            unsafe { env::remove_var(format!("{tool}_{target}")) };
        }
        config.target("aarch64-linux-android");
        // --host 必须显式给：autotools 包从 CC 文件名剥 -cc/-gcc 后缀推导，
        // NDK 包装叫 aarch64-linux-android26-clang，剥不出东西，configure 会按
        // 主机编（编出 x86_64 的 .a，链接期才炸）。config_option 直出 --host。
        config.config_option("host", Some("aarch64-linux-android"));
        config.disable("fontconfig", None);
        // fontconfig 关掉后 configure 会以“没有系统字体提供器”拒绝，
        // 必须再加这个（configure 原文提示）。后果：手机上 ass_set_fonts
        // 找不到系统字体，只能用内嵌字体 + 后续自带的 fonts.conf；
        // 字幕能出字，字体回退先降级，见台账。
        config.disable("require-system-font-provider", None);
    }
    let dst = config.build();

    // Compile our C wrapper file.
    cc::Build::new()
        .file("wrapper.c")
        .include(format!("{}/include", dst.display()))
        .opt_level(1)
        .compile("wrapper");

    // Linker instructions for Cargo.
    println!("cargo:rustc-link-search=native={}/lib", dst.display());
    println!("cargo:rustc-link-lib=static=ass");

    // Probe for other required system libraries.
    // Android 上 fontconfig 已在上面禁用，这里跳过它的探测；
    // 其余三项走 TARGET_PKG_CONFIG_PATH（android-deps.sh 产物，flake 导出）。
    pkg_config::Config::new().probe("freetype2").unwrap();
    pkg_config::Config::new().probe("fribidi").unwrap();
    pkg_config::Config::new().probe("harfbuzz").unwrap();
    if !is_android {
        pkg_config::Config::new().probe("fontconfig").unwrap();
    }

    // 2. Generate bindings on the fly, but with our custom callback.
    let out_path = PathBuf::from(env::var("OUT_DIR").unwrap());
    let mut builder = bindgen::Builder::default()
        .header("wrapper.h")
        .clang_arg(format!("-I{}/include", dst.display()));
    // Android：bindgen 跑的是主机 clang，不给目标它就去啃主机 glibc 头
    //（曾挂在 gnu/stubs-32.h 缺失）。指到 NDK sysroot + 三元组，
    // 解析 bionic 头；ass 头本身是平台无关的，绑定结果正确。
    if is_android {
        let ndk = env::var("ANDROID_NDK_ROOT").expect("flake 的 shellHook 导出 ANDROID_NDK_ROOT");
        let sysroot = format!("{}/toolchains/llvm/prebuilt/linux-x86_64/sysroot", ndk);
        builder = builder
            .clang_arg("--target=aarch64-linux-android26")
            .clang_arg(format!("--sysroot={}", sysroot))
            // bindgen 直调 libclang，走不到驱动层的 --sysroot 解析（实测主机
            // glibc 头仍被啃），再加显式 -isystem 把标准头钉死在 NDK 里。
            // 顺序照抄 NDK 驱动：arch 专属在前，通用在后；写反了会啃到 32 位
            // 的 asm/posix_types.h，把 ssize_t 解析成 4 字节（实测踩过）。
            .clang_arg(format!("-isystem{}/usr/include/aarch64-linux-android", sysroot))
            .clang_arg(format!("-isystem{}/usr/include", sysroot));
    }
    let bindings = builder
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
