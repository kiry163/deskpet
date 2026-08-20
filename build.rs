//! 构建脚本：定位 libvpx 静态库（vpx.lib / vpxmd.lib）并输出链接指令。
//!
//! libvpx 解析优先级：VPX_LIB_DIR 环境变量 > VCPKG_ROOT/installed/x64-windows/lib > vendor_libvpx/x64/Release（上游约定路径）。
//! 素材不再在构建期打包（素材与软件分离，运行时从目录加载）。

use std::env;
use std::path::{Path, PathBuf};

fn main() {
    // Windows：嵌入 exe 图标资源（resources/deskpet.rc → deskpet.ico）
    #[cfg(windows)]
    embed_resource::compile("resources/deskpet.rc", embed_resource::NONE);

    let manifest = PathBuf::from(env::var("CARGO_MANIFEST_DIR").unwrap());
    let target = env::var("TARGET").unwrap_or_default();
    if target.contains("apple-darwin") {
        link_libvpx_macos();
    } else {
        link_libvpx_windows(&manifest);
    }
}

fn link_libvpx_windows(manifest: &Path) {
    println!("cargo:rerun-if-env-changed=VPX_LIB_DIR");
    println!("cargo:rerun-if-env-changed=VCPKG_ROOT");

    let mut candidates: Vec<PathBuf> = Vec::new();
    if let Ok(dir) = env::var("VPX_LIB_DIR") {
        if !dir.trim().is_empty() {
            candidates.push(PathBuf::from(dir));
        }
    }
    if let Ok(root) = env::var("VCPKG_ROOT") {
        candidates.push(PathBuf::from(root).join("installed").join("x64-windows").join("lib"));
    }
    // 上游约定路径（按 README 构建 libvpx 后产物在此）
    candidates.push(manifest.join("vendor_libvpx").join("x64").join("Release"));

    for dir in &candidates {
        if !dir.is_dir() {
            continue;
        }
        if dir.join("vpx.lib").is_file() {
            println!("cargo:rustc-link-search=native={}", dir.display());
            println!("cargo:rustc-link-lib=static=vpx");
            return;
        }
        if dir.join("vpxmd.lib").is_file() {
            println!("cargo:rustc-link-search=native={}", dir.display());
            println!("cargo:rustc-link-lib=static=vpxmd");
            return;
        }
    }
    // 仅做 cargo check 时（DESKPET_ALLOW_NO_LIBVPX=1）缺库不报错：check 不链接，
    // 真实构建/链接会以 "cannot find -lvpx" 明确报错。
    println!("cargo:warning=libvpx 未找到。若为 cargo check 可忽略；真实构建请先安装 libvpx 并设置 VPX_LIB_DIR。");
    if env::var("DESKPET_ALLOW_NO_LIBVPX").is_ok() {
        return;
    }
    panic!(
        "找不到 libvpx 静态库（vpx.lib / vpxmd.lib）。\
         请先安装：vcpkg install libvpx:x64-windows，然后设置 VPX_LIB_DIR 指向其 lib 目录；\
         或按上游方式在 vendor_libvpx/x64/Release 放置 vpxmd.lib。\
         （仅做 cargo check 时可用 DESKPET_ALLOW_NO_LIBVPX=1 跳过）"
    );
}

/// macOS：homebrew / vcpkg 或 VPX_LIB_DIR 中的 libvpx（静态 .a 优先，其次动态 .dylib）。
fn link_libvpx_macos() {
    println!("cargo:rerun-if-env-changed=VPX_LIB_DIR");
    println!("cargo:rerun-if-env-changed=VCPKG_ROOT");

    let mut candidates: Vec<PathBuf> = Vec::new();
    if let Ok(dir) = env::var("VPX_LIB_DIR") {
        if !dir.trim().is_empty() {
            candidates.push(PathBuf::from(dir));
        }
    }
    if let Ok(root) = env::var("VCPKG_ROOT") {
        let root = PathBuf::from(root);
        candidates.push(root.join("installed").join("arm64-osx").join("lib"));
        candidates.push(root.join("installed").join("x64-osx").join("lib"));
    }
    // homebrew（Apple Silicon / Intel）
    candidates.push(PathBuf::from("/opt/homebrew/lib"));
    candidates.push(PathBuf::from("/usr/local/lib"));

    for dir in &candidates {
        if !dir.is_dir() {
            continue;
        }
        if dir.join("libvpx.a").is_file() {
            println!("cargo:rustc-link-search=native={}", dir.display());
            // macOS 的 ld 在同目录同时存在 libvpx.a / libvpx.dylib 时，`-l vpx`
            // 会优先选择 dylib（rustc 的 static=vpx 不强制归档），导致产物动态
            // 依赖 brew 的 libvpx。这里直接把归档绝对路径作为链接输入，
            // 强制静态链接，保证单文件自包含（与 Windows 行为一致）。
            println!("cargo:rustc-link-arg-bins={}", dir.join("libvpx.a").display());
            return;
        }
        if dir.join("libvpx.dylib").is_file() {
            println!("cargo:rustc-link-search=native={}", dir.display());
            println!("cargo:rustc-link-lib=vpx");
            return;
        }
    }
    println!("cargo:warning=libvpx 未找到（macOS）。若为 cargo check 可忽略；真实构建请先安装 libvpx 并设置 VPX_LIB_DIR。");
    if env::var("DESKPET_ALLOW_NO_LIBVPX").is_ok() {
        return;
    }
    panic!(
        "找不到 libvpx 库（libvpx.a / libvpx.dylib）。\
         请先安装：brew install libvpx（或 vcpkg install libvpx），\
         或用 VPX_LIB_DIR 指向其 lib 目录。\
         （仅做 cargo check 时可用 DESKPET_ALLOW_NO_LIBVPX=1 跳过）"
    );
}
