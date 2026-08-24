//! 版本显示与更新检查。
//!
//! 更新源是**可配置的 JSON 清单地址**，没配就报「未配置」而不是假装检查成功。
//! 清单格式（自己托管一个静态文件即可）：
//! ```json
//! { "version": "0.2.0", "url": "https://.../FireVibe-0.2.0.dmg", "notes": "..." }
//! ```

use serde::Deserialize;
use std::time::Duration;

/// 当前版本，编译期从 Cargo.toml 取
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

/// 默认更新源：GitHub Releases API。`/releases/latest` 只返回**非预发布**的那个，
/// 正好是 app 正式版（cli 那些是 prerelease，会被忽略）。不用自己托管清单。
const GITHUB_LATEST: &str = "https://api.github.com/repos/tankxu/firevibe/releases/latest";
/// 用户下载页（有新版时跳这里）
pub const RELEASES_PAGE: &str = "https://github.com/tankxu/firevibe/releases/latest";

#[derive(Clone, Debug, PartialEq)]
pub enum UpdateStatus {
    /// 还没查过
    Idle,
    Checking,
    /// 已是最新
    UpToDate,
    /// 有新版本
    Available {
        version: String,
        url: String,
        notes: String,
    },
    /// 没配更新源
    NotConfigured,
    Failed(String),
}

impl UpdateStatus {
    pub fn label(&self) -> String {
        match self {
            UpdateStatus::Idle => "未检查".into(),
            UpdateStatus::Checking => "检查中…".into(),
            UpdateStatus::UpToDate => "已是最新".into(),
            UpdateStatus::Available { version, .. } => format!("有新版本 {version}"),
            UpdateStatus::NotConfigured => "未配置更新源".into(),
            UpdateStatus::Failed(e) => format!("检查失败: {e}"),
        }
    }
    pub fn has_update(&self) -> bool {
        matches!(self, UpdateStatus::Available { .. })
    }
}

#[derive(Deserialize)]
struct Manifest {
    version: String,
    #[serde(default)]
    url: String,
    #[serde(default)]
    notes: String,
}

/// 比较语义化版本，只看 major.minor.patch，非数字段忽略
fn newer(remote: &str, local: &str) -> bool {
    let parse = |s: &str| -> Vec<u32> {
        s.split(['.', '-', '+'])
            .filter_map(|x| x.parse::<u32>().ok())
            .take(3)
            .collect()
    };
    let (r, l) = (parse(remote), parse(local));
    for i in 0..3 {
        let a = r.get(i).copied().unwrap_or(0);
        let b = l.get(i).copied().unwrap_or(0);
        if a != b {
            return a > b;
        }
    }
    false
}

/// 同步检查更新（调用方自己丢线程里跑）。
/// endpoint 非空 = 用自定义 JSON 清单；空 = 默认查 GitHub Releases。
pub fn check(endpoint: Option<&str>) -> UpdateStatus {
    match endpoint.map(|s| s.trim()).filter(|s| !s.is_empty()) {
        Some(url) => check_manifest(url),
        None => check_github(),
    }
}

fn http_get(url: &str) -> Result<String, String> {
    let resp = ureq::AgentBuilder::new()
        .timeout(Duration::from_secs(8))
        .build()
        .get(url)
        // GitHub API 强制要 User-Agent，没有会 403
        .set("User-Agent", "FireVibe")
        .set("Accept", "application/vnd.github+json")
        .call();
    match resp {
        Ok(r) => r.into_string().map_err(|e| e.to_string()),
        Err(e) => Err(short_err(&e.to_string())),
    }
}

/// 自定义 JSON 清单：{ "version", "url", "notes" }
fn check_manifest(url: &str) -> UpdateStatus {
    let body = match http_get(url) {
        Ok(b) => b,
        Err(e) => return UpdateStatus::Failed(e),
    };
    let m: Manifest = match serde_json::from_str(&body) {
        Ok(m) => m,
        Err(e) => return UpdateStatus::Failed(format!("清单解析失败: {e}")),
    };
    if newer(&m.version, VERSION) {
        UpdateStatus::Available { version: m.version, url: m.url, notes: m.notes }
    } else {
        UpdateStatus::UpToDate
    }
}

#[derive(Deserialize)]
struct GhRelease {
    tag_name: String,
    #[serde(default)]
    html_url: String,
    #[serde(default)]
    body: String,
}

/// 默认：GitHub Releases/latest（tag 形如 v0.1.2）
fn check_github() -> UpdateStatus {
    let body = match http_get(GITHUB_LATEST) {
        Ok(b) => b,
        Err(e) => return UpdateStatus::Failed(e),
    };
    let g: GhRelease = match serde_json::from_str(&body) {
        Ok(g) => g,
        Err(e) => return UpdateStatus::Failed(format!("GitHub 响应解析失败: {e}")),
    };
    let ver = g.tag_name.trim_start_matches(['v', 'V']).to_string();
    if newer(&ver, VERSION) {
        let url = if g.html_url.is_empty() { RELEASES_PAGE.to_string() } else { g.html_url };
        let notes: String = g.body.lines().take(12).collect::<Vec<_>>().join("\n");
        UpdateStatus::Available { version: ver, url, notes }
    } else {
        UpdateStatus::UpToDate
    }
}

fn short_err(s: &str) -> String {
    s.chars().take(80).collect()
}

#[cfg(test)]
mod tests {
    use super::newer;
    #[test]
    fn version_compare() {
        assert!(newer("0.2.0", "0.1.0"));
        assert!(newer("1.0.0", "0.9.9"));
        assert!(newer("0.1.2", "0.1.1"));
        assert!(!newer("0.1.0", "0.1.0"));
        assert!(!newer("0.1.0", "0.2.0"));
        assert!(newer("0.2.0-beta", "0.1.9"));
    }
}
