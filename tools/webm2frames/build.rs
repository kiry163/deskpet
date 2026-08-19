//! 构建配置：定位 Homebrew libvpx 的库搜索路径。
//! （vpx.rs 通过 #[link(name = "vpx")] 链接 libvpx，但 brew 的库不在系统默认路径，
//!   需要在链接器搜索路径中加入 brew prefix 的 lib 目录。）

fn main() {
    // Intel Mac: /usr/local；Apple Silicon: /opt/homebrew
    let brew_prefix = std::env::var("HOMEBREW_PREFIX").unwrap_or_else(|_| "/usr/local".to_string());
    println!("cargo:rustc-link-search={}/lib", brew_prefix);
    println!("cargo:rerun-if-changed=build.rs");
}
