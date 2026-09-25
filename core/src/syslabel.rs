//! 系统设置里某几项的**当前显示名** —— 直接问系统要，不在文案里写死。
//!
//! 起因：macOS 27 把隐私里 `kTCCServiceAccessibility` 那一项改了名 ——
//! 英文 `Accessibility` → `Device Control and Data Access`，
//! 中文「辅助功能」→「设备控制和数据访问」。内部标识一个没变（URL 锚点还是
//! `Privacy_Accessibility`、`tccutil` 服务名还是 `Accessibility`、API 还是
//! `AXIsProcessTrusted`），变的只有给用户看的那个词。我们的文案写死旧名，
//! 用户在设置里**找不到那一项** —— 真卡住过。
//!
//! 解法：隐私面板自己的本地化表里就有这个字符串，key 是 `ACCESSIBILITY`。
//! 读它 = 永远和用户屏幕上一致，系统以后再改名也自动跟上，**且不用猜改名发生在
//! 哪个大版本**（阈值猜错就又错一轮）。
//!
//! ⚠️ 用**系统界面语言**取，不是 app 的界面语言：这个词的唯一用途是让用户在系统
//! 设置里找到那一行，必须和他屏幕上的字一模一样。app 设成中文、系统是英文时，
//! 给中文名反而找不到。
//!
//! ⚠️ 别去查 `AccessibilitySettingsExtension.appex` —— 那是 VoiceOver/缩放那个
//! *辅助功能设置面板*（它叫「无障碍」），和隐私里这一行是两回事。

use std::process::Command;
use std::sync::OnceLock;

const LOCTABLE: &str = "/System/Library/ExtensionKit/Extensions/\
                        SecurityPrivacyExtension.appex/Contents/Resources/Localizable.loctable";

/// `defaults read -g AppleLanguages` 的第一条 = 系统界面语言（如 `en-US`、`zh-Hans-CN`）。
/// ⚠️ 不用 `AppleLocale`：那是地区/格式，这台机器上读出来是 `en_US@rg=sgzzzz`，没法直接当 key。
fn system_language() -> Option<String> {
    let out = Command::new("/usr/bin/defaults")
        .args(["read", "-g", "AppleLanguages"])
        .output()
        .ok()?;
    String::from_utf8(out.stdout)
        .ok()?
        .lines()
        .map(|l| l.trim().trim_end_matches(',').trim_matches('"').to_string())
        .find(|l| !l.is_empty() && l != "(" && l != ")")
}

/// 语言标签 → loctable 里可能的 key，按优先级排。表里是 `en` / `zh_CN` / `zh_TW` / `ja` 这种。
fn keys_for(lang: &str) -> Vec<String> {
    let mut v = Vec::new();
    // 中文按字形分，不能只看主语言子标签：表里没有裸 `zh`
    if lang.starts_with("zh") {
        v.push(if lang.contains("Hant") {
            if lang.contains("HK") { "zh_HK" } else { "zh_TW" }
        } else {
            "zh_CN"
        }
        .to_string());
    }
    v.push(lang.replace('-', "_")); // en-US -> en_US
    if let Some(primary) = lang.split(['-', '_']).next() {
        v.push(primary.to_string()); // en-US -> en
    }
    v.push("en".to_string()); // 兜底
    v
}

fn read(key: &str) -> Option<String> {
    let out = Command::new("/usr/bin/plutil")
        .args(["-extract", &format!("{key}.ACCESSIBILITY"), "raw", "-o", "-", LOCTABLE])
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let s = String::from_utf8(out.stdout).ok()?.trim().to_string();
    // 一行、不长才采信 —— 万一这个 key 的值哪天变成一段说明文字，别把它塞进按钮标题
    (!s.is_empty() && s.chars().count() <= 64 && !s.contains('\n')).then_some(s)
}

/// 「辅助功能 / Device Control and Data Access」这一项在当前系统上的显示名。
/// 读不到返回 `None`（老系统没有这个 appex），调用方用自己的兜底文案。
pub fn accessibility() -> Option<&'static str> {
    // 进程内只探一次；系统语言中途改了要重开 app，和系统设置里改语言的行为一致。
    static V: OnceLock<Option<String>> = OnceLock::new();
    V.get_or_init(|| {
        let lang = system_language().unwrap_or_else(|| "en".to_string());
        keys_for(&lang).into_iter().find_map(|k| read(&k))
    })
    .as_deref()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 老系统上没有这个 appex，读不到是正常的；但**表在却读不出来**就是路径或 key
    /// 写错了 —— 那种错会静默退回旧名，正是这个模块要解决的问题本身。
    #[test]
    fn label_matches_system() {
        let has_table = std::path::Path::new(LOCTABLE).is_file();
        match accessibility() {
            Some(s) => assert!(
                !s.is_empty() && s.chars().count() <= 64 && !s.contains('\n'),
                "读出来的不像一个名字: {s:?}"
            ),
            None => assert!(!has_table, "本地化表存在却没读出名字 —— 路径或 key 写错了：{LOCTABLE}"),
        }
    }

    /// 中文必须按字形映射到 zh_CN / zh_TW / zh_HK —— 表里没有裸 `zh`，
    /// 只取主语言子标签的话简体用户会掉到英文兜底。
    #[test]
    fn chinese_maps_by_script() {
        assert_eq!(keys_for("zh-Hans-CN")[0], "zh_CN");
        assert_eq!(keys_for("zh-Hant-TW")[0], "zh_TW");
        assert_eq!(keys_for("zh-Hant-HK")[0], "zh_HK");
        assert_eq!(keys_for("en-US")[0], "en_US");
        assert!(keys_for("en-US").contains(&"en".to_string()));
    }
}
