//! 按键合成。取代 Karabiner —— 每个平台一套原生实现。

use anyhow::Result;

pub trait Injector: Send + Sync {
    fn available(&self) -> bool;
    /// 不可用时说明原因与解决办法
    fn why(&self) -> String;
    /// 按下再松开，一次完整敲击
    fn key_stroke(&self, key: &str, mods: &[String]) -> Result<()>;
    /// 只按下不松开。用来伺候「按住说话」型的外部语音 app ——
    /// 那类 app 靠快捷键的按住时长判断录音区间，一体式敲击对它没用。
    fn key_down(&self, key: &str, mods: &[String]) -> Result<()>;
    /// 松开
    fn key_up(&self, key: &str, mods: &[String]) -> Result<()>;
    fn type_text(&self, s: &str) -> Result<()>;
}

#[cfg(target_os = "macos")]
mod macos;
#[cfg(target_os = "macos")]
pub use macos::{key_names, new_injector, ns_modifier_alt};

// Linux(uinput) / Windows(SendInput) 还没写。之前这里是 `mod linux;` / `mod windows;`
// 但文件从来就不存在 —— 非 macOS 平台压根编译不过。先统一落到 fallback：
// 别的都能用，只有按键注入报「这个平台没有按键注入」。
#[cfg(not(target_os = "macos"))]
mod fallback;
#[cfg(not(target_os = "macos"))]
pub use fallback::{key_names, new_injector};
