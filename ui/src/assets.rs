//! 图标资源。
//!
//! gpui-component 的 `IconName::path()` 返回的是 `"icons/settings.svg"` 这种
//! **相对资源包的路径**，crate 本身不带 svg 文件 —— 必须由 app 通过
//! `Application::with_assets` 提供，否则所有图标都是空白。
//! 这些 svg 取自 longbridge/gpui-component 的 crates/assets/assets/icons。

use anyhow::Result;
use gpui::{AssetSource, SharedString};
use include_dir::{include_dir, Dir};
use std::borrow::Cow;

// ⚠️ `include_dir!` 是编译期整目录嵌入，但它**不声明 rerun-if-changed**：
// 往 assets/icons/ 里新加 svg 不会触发重编，编出来的还是旧的那套图标
// （表现是新图标位置一片空白）。加完图标要 `touch ui/src/assets.rs` 再构建。
static ICONS: Dir<'_> = include_dir!("$CARGO_MANIFEST_DIR/assets/icons");

pub struct Assets;

impl AssetSource for Assets {
    fn load(&self, path: &str) -> Result<Option<Cow<'static, [u8]>>> {
        // 传进来的是 "icons/xxx.svg"，剥掉前缀去目录里找
        let rel = path.strip_prefix("icons/").unwrap_or(path);
        Ok(ICONS
            .get_file(rel)
            .map(|f| Cow::Borrowed(f.contents())))
    }

    fn list(&self, _path: &str) -> Result<Vec<SharedString>> {
        Ok(ICONS
            .files()
            .map(|f| SharedString::from(format!("icons/{}", f.path().display())))
            .collect())
    }
}
