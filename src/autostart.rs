//! 开机自启。
//! Windows：HKCU\Software\Microsoft\Windows\CurrentVersion\Run 注册表键；
//! macOS：~/Library/LaunchAgents/com.kiry.deskpet.plist（LaunchAgent）。
#![allow(dead_code)]

// ---------------- Windows ----------------

#[cfg(windows)]
mod win32_autostart {
    use windows_sys::Win32::{
        Foundation::GetLastError,
        System::{
            LibraryLoader::GetModuleFileNameW,
            Registry::{
                RegOpenKeyExW, RegSetValueExW, RegDeleteValueW, RegQueryValueExW, HKEY,
                HKEY_CURRENT_USER, KEY_SET_VALUE, KEY_QUERY_VALUE, REG_SZ,
            },
        },
    };

    const RUN_KEY: &str = "Software\\Microsoft\\Windows\\CurrentVersion\\Run";
    const VALUE_NAME: &str = "deskpet";

    fn run_key_path() -> Vec<u16> {
        RUN_KEY.encode_utf16().chain(std::iter::once(0)).collect()
    }

    fn value_name() -> Vec<u16> {
        VALUE_NAME.encode_utf16().chain(std::iter::once(0)).collect()
    }

    /// 当前 exe 路径。
    fn exe_path() -> String {
        use std::ffi::OsString;
        use std::os::windows::ffi::OsStringExt;
        let mut buf = vec![0u16; 4096];
        let len = unsafe { GetModuleFileNameW(std::ptr::null_mut(), buf.as_mut_ptr(), 4096) };
        buf.truncate(len as usize);
        let os = OsString::from_wide(&buf);
        os.to_string_lossy().to_string()
    }

    pub fn is_enabled() -> bool {
        let path = run_key_path();
        let mut key: HKEY = std::ptr::null_mut();
        let status = unsafe {
            RegOpenKeyExW(HKEY_CURRENT_USER, path.as_ptr(), 0, KEY_QUERY_VALUE, &mut key)
        };
        if status != 0 {
            log_debug!("开机自启: 关（注册表键不存在）");
            return false;
        }
        let name = value_name();
        let mut buf = [0u16; 1024];
        let mut size: u32 = (buf.len() * 2) as u32;
        let r = unsafe {
            RegQueryValueExW(
                key,
                name.as_ptr(),
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                buf.as_mut_ptr() as *mut u8,
                &mut size,
            )
        };
        let _ = unsafe { windows_sys::Win32::System::Registry::RegCloseKey(key) };
        log_debug!("开机自启: {}", if r == 0 { "开" } else { "关" });
        r == 0
    }

    pub fn set_enabled(on: bool) {
        if on {
            enable();
        } else {
            disable();
        }
    }

    fn enable() {
        let path = run_key_path();
        let mut key: HKEY = std::ptr::null_mut();
        let status = unsafe {
            RegOpenKeyExW(HKEY_CURRENT_USER, path.as_ptr(), 0, KEY_SET_VALUE, &mut key)
        };
        if status != 0 {
            log_warn!("开启开机自启失败（打开注册表键失败, status={:#x}）", status);
            return;
        }
        let cmd: Vec<u16> = format!("\"{}\"", exe_path())
            .encode_utf16()
            .chain(std::iter::once(0))
            .collect();
        unsafe {
            RegSetValueExW(
                key,
                value_name().as_ptr(),
                0,
                REG_SZ,
                cmd.as_ptr() as *const u8,
                (cmd.len() * 2) as u32,
            );
            windows_sys::Win32::System::Registry::RegCloseKey(key);
        }
        log_info!("已开启开机自启: {}", exe_path());
    }

    fn disable() {
        let path = run_key_path();
        let mut key: HKEY = std::ptr::null_mut();
        let status = unsafe {
            RegOpenKeyExW(HKEY_CURRENT_USER, path.as_ptr(), 0, KEY_SET_VALUE, &mut key)
        };
        if status != 0 {
            log_warn!("关闭开机自启失败（打开注册表键失败, status={:#x}）", status);
            return;
        }
        unsafe {
            RegDeleteValueW(key, value_name().as_ptr());
            windows_sys::Win32::System::Registry::RegCloseKey(key);
        }
        let _ = GetLastError;
        log_info!("已关闭开机自启");
    }
}

// ---------------- macOS ----------------

#[cfg(target_os = "macos")]
mod macos_autostart {
    use std::path::PathBuf;

    fn plist_path() -> PathBuf {
        let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
        PathBuf::from(home)
            .join("Library")
            .join("LaunchAgents")
            .join("com.kiry.deskpet.plist")
    }

    pub fn is_enabled() -> bool {
        let r = plist_path().is_file();
        log_debug!("开机自启: {}", if r { "开" } else { "关" });
        r
    }

    pub fn set_enabled(on: bool) {
        if on {
            enable();
        } else {
            disable();
        }
    }

    fn enable() {
        let Some(exe) = std::env::current_exe().ok() else { return };
        let path = plist_path();
        if let Some(dir) = path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        let plist = format!(
            "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n\
             <!DOCTYPE plist PUBLIC \"-//Apple//DTD PLIST 1.0//EN\" \"http://www.apple.com/DTDs/PropertyList-1.0.dtd\">\n\
             <plist version=\"1.0\">\n\
             <dict>\n\
               <key>Label</key><string>com.kiry.deskpet</string>\n\
               <key>ProgramArguments</key>\n\
               <array><string>{}</string></array>\n\
               <key>RunAtLoad</key><true/>\n\
             </dict>\n\
             </plist>\n",
            exe.display()
        );
        let _ = std::fs::write(&path, plist);
        log_info!("已开启开机自启: {}", path.display());
    }

    fn disable() {
        let path = plist_path();
        let _ = std::fs::remove_file(&path);
        log_info!("已关闭开机自启");
    }
}

#[cfg(windows)]
pub use win32_autostart::{is_enabled, set_enabled};
#[cfg(target_os = "macos")]
pub use macos_autostart::{is_enabled, set_enabled};
