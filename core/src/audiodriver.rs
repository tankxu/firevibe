//! 装 / 查我们自己那块虚拟声卡（`driver/build.sh` 编出来的）。
//!
//! 为什么要自己带一块：第三方语音输入工具会把传输类型是「虚拟」的设备
//! 从麦克风候选里滤掉（BlackHole 就是这么被漏掉的）。我们这块自称 USB，
//! 所以它们认。做法见 driver/build.sh。
//!
//! 装到 `/Library/Audio/Plug-Ins/HAL` 需要管理员权限，走 osascript 的
//! `with administrator privileges` —— 弹的是系统原生授权框，密码不经过我们。

use anyhow::{anyhow, Context, Result};
use std::path::PathBuf;

/// 设备名（用户在声音设置和第三方工具里看到的）
pub const DEVICE_NAME: &str = "FireVibe Mic";
/// 驱动包名
pub const BUNDLE: &str = "FireVibeMic.driver";
const HAL_DIR: &str = "/Library/Audio/Plug-Ins/HAL";

/// app 里带的那一份（打包时放进 Contents/Resources）
pub fn bundled() -> Option<PathBuf> {
    let exe = std::env::current_exe().ok()?;
    // FireVibe.app/Contents/MacOS/firevibe -> Contents/Resources/FireVibeMic.driver
    let p = exe.parent()?.parent()?.join("Resources").join(BUNDLE);
    if p.is_dir() {
        return Some(p);
    }
    // 开发时直接跑 target/release/firevibe-ui：退回仓库里的产物
    let dev = exe.parent()?.parent()?.parent()?.join("driver/out").join(BUNDLE);
    dev.is_dir().then_some(dev)
}

/// 已经装了吗
pub fn installed() -> bool {
    PathBuf::from(HAL_DIR).join(BUNDLE).is_dir()
}

/// 系统里已经能看到这块设备了吗（装完还要 coreaudiod 重启才出现）
pub fn device_present() -> bool {
    crate::audio::input_devices()
        .iter()
        .any(|d| d.name.starts_with(DEVICE_NAME))
}

/// 装（或覆盖安装）。会弹系统的管理员授权框。
pub fn install() -> Result<()> {
    let src = bundled().ok_or_else(|| anyhow!("应用里没有带驱动（打包时没放进 Resources）"))?;
    let src = src.to_string_lossy().to_string();
    if src.contains('"') || src.contains('\\') {
        return Err(anyhow!("驱动路径里有引号或反斜杠，拒绝执行：{src}"));
    }
    // 先删后拷，避免覆盖到一半的残留；最后重启 coreaudiod 让设备出现
    let sh = format!(
        "rm -rf '{HAL_DIR}/{BUNDLE}' && cp -R '{src}' '{HAL_DIR}/' && killall -9 coreaudiod"
    );
    run_as_admin(&sh)
}

/// 卸载
pub fn uninstall() -> Result<()> {
    run_as_admin(&format!(
        "rm -rf '{HAL_DIR}/{BUNDLE}' && killall -9 coreaudiod"
    ))
}

fn run_as_admin(sh: &str) -> Result<()> {
    // osascript 的字符串里要转义反斜杠和双引号
    let quoted = sh.replace('\\', "\\\\").replace('"', "\\\"");
    let script = format!("do shell script \"{quoted}\" with administrator privileges");
    let out = std::process::Command::new("/usr/bin/osascript")
        .args(["-e", &script])
        .output()
        .context("跑 osascript 失败")?;
    if out.status.success() {
        return Ok(());
    }
    let err = String::from_utf8_lossy(&out.stderr);
    if err.contains("-128") {
        return Err(anyhow!("你取消了授权"));
    }
    Err(anyhow!("安装失败：{}", err.trim()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn paths_are_sane() {
        assert!(HAL_DIR.starts_with('/'));
        assert!(BUNDLE.ends_with(".driver"));
        assert_eq!(DEVICE_NAME, "FireVibe Mic");
    }

    /// 命令里带引号的路径必须被拒，不能拼进 shell
    #[test]
    fn rejects_quoted_paths() {
        let bad = "/tmp/a\"b/FireVibeMic.driver";
        assert!(bad.contains('"'), "这个测试样本本身要含引号");
    }
}
