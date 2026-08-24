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

/// 同步检查更新（调用方自己丢线程里跑）
pub fn check(endpoint: Option<&str>) -> UpdateStatus {
    let Some(url) = endpoint.filter(|u| !u.trim().is_empty()) else {
        return UpdateStatus::NotConfigured;
    };
    let resp = ureq::AgentBuilder::new()
        .timeout(Duration::from_secs(8))
        .build()
        .get(url)
        .call();
    let body = match resp {
        Ok(r) => match r.into_string() {
            Ok(b) => b,
            Err(e) => return UpdateStatus::Failed(e.to_string()),
        },
        Err(e) => return UpdateStatus::Failed(short_err(&e.to_string())),
    };
    let m: Manifest = match serde_json::from_str(&body) {
        Ok(m) => m,
        Err(e) => return UpdateStatus::Failed(format!("清单解析失败: {e}")),
    };
    if newer(&m.version, VERSION) {
        UpdateStatus::Available {
            version: m.version,
            url: m.url,
            notes: m.notes,
        }
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
