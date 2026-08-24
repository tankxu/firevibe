//! 开机启动。macOS 走用户级 LaunchAgent，其它平台先明确报「不支持」，
//! 不假装成功 —— 界面上的开关拨回去比默默无效好。

use anyhow::{anyhow, Result};

pub const LABEL: &str = "com.tankxu.firevibe";

#[cfg(target_os = "macos")]
mod imp {
    use super::*;
    use std::path::PathBuf;

    fn plist_path() -> Result<PathBuf> {
        let home = dirs::home_dir().ok_or_else(|| anyhow!("找不到 home 目录"))?;
        Ok(home
            .join("Library/LaunchAgents")
            .join(format!("{LABEL}.plist")))
    }

    pub fn enabled() -> bool {
        plist_path().map(|p| p.exists()).unwrap_or(false)
    }

    pub fn set(on: bool) -> Result<()> {
        let path = plist_path()?;
        if !on {
            if path.exists() {
                // 先卸载再删，免得留个僵尸 job
                let _ = std::process::Command::new("launchctl")
                    .args([
                        "bootout",
                        &format!("gui/{}", uid()),
                        path.to_str().unwrap_or(""),
                    ])
                    .status();
                std::fs::remove_file(&path)?;
            }
            return Ok(());
        }
        let exe = std::env::current_exe()?;
        let exe = exe.to_string_lossy();
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        // 界面型 app 交给 launchd 直接跑二进制即可；RunAtLoad 只在登录时拉一次
        let plist = format!(
            r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>Label</key><string>{LABEL}</string>
  <key>ProgramArguments</key>
  <array><string>{exe}</string></array>
  <key>RunAtLoad</key><true/>
  <key>KeepAlive</key><false/>
  <key>ProcessType</key><string>Interactive</string>
</dict>
</plist>
"#
        );
        std::fs::write(&path, plist)?;
        let _ = std::process::Command::new("launchctl")
            .args([
                "bootstrap",
                &format!("gui/{}", uid()),
                path.to_str().unwrap_or(""),
            ])
            .status();
        Ok(())
    }

    fn uid() -> u32 {
        // SAFETY: getuid 永远成功，没有副作用
        unsafe { libc::getuid() }
    }
}

#[cfg(not(target_os = "macos"))]
mod imp {
    use super::*;
    pub fn enabled() -> bool {
        false
    }
    pub fn set(_on: bool) -> Result<()> {
        Err(anyhow!("这个平台还没做开机启动"))
    }
}

pub use imp::{enabled, set};
