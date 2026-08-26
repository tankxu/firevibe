//! macOS 原生文件选择器。配置导入/导出必须让用户明确选择路径，不能在 UI
//! 线程外操作 AppKit，也不该依赖 shell 脚本拼对话框。

use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver};

#[cfg(target_os = "macos")]
fn url_path(url: &objc2_foundation::NSURL) -> Option<PathBuf> {
    Some(PathBuf::from(url.path()?.to_string()))
}

/// 异步选择一个要导入的配置文件。通道收到 `None` 表示取消。
#[cfg(target_os = "macos")]
pub fn pick_import(title: &str, prompt: &str) -> Receiver<Option<PathBuf>> {
    use block2::RcBlock;
    use objc2::MainThreadMarker;
    use objc2_app_kit::{NSModalResponseOK, NSOpenPanel};
    use objc2_foundation::NSString;

    let (tx, rx) = mpsc::channel();
    let Some(mtm) = MainThreadMarker::new() else {
        let _ = tx.send(None);
        return rx;
    };
    let panel = NSOpenPanel::openPanel(mtm);
    panel.setCanChooseFiles(true);
    panel.setCanChooseDirectories(false);
    panel.setAllowsMultipleSelection(false);
    let title = NSString::from_str(title);
    let prompt = NSString::from_str(prompt);
    panel.setTitle(Some(&title));
    panel.setPrompt(Some(&prompt));
    let panel_for_result = panel.clone();
    let handler = RcBlock::new(move |response| {
        let path = (response == NSModalResponseOK)
            .then(|| panel_for_result.URL())
            .flatten()
            .and_then(|url| url_path(&url));
        let _ = tx.send(path);
    });
    panel.beginWithCompletionHandler(&handler);
    rx
}

/// 异步选择配置导出的目标路径。通道收到 `None` 表示取消。
#[cfg(target_os = "macos")]
pub fn pick_export(title: &str, prompt: &str, default_name: &str) -> Receiver<Option<PathBuf>> {
    use block2::RcBlock;
    use objc2::MainThreadMarker;
    use objc2_app_kit::{NSModalResponseOK, NSSavePanel};
    use objc2_foundation::NSString;

    let (tx, rx) = mpsc::channel();
    let Some(mtm) = MainThreadMarker::new() else {
        let _ = tx.send(None);
        return rx;
    };
    let panel = NSSavePanel::savePanel(mtm);
    let title = NSString::from_str(title);
    let prompt = NSString::from_str(prompt);
    let name = NSString::from_str(default_name);
    panel.setTitle(Some(&title));
    panel.setPrompt(Some(&prompt));
    panel.setNameFieldStringValue(&name);
    let panel_for_result = panel.clone();
    let handler = RcBlock::new(move |response| {
        let path = (response == NSModalResponseOK)
            .then(|| panel_for_result.URL())
            .flatten()
            .and_then(|url| url_path(&url));
        let _ = tx.send(path);
    });
    panel.beginWithCompletionHandler(&handler);
    rx
}

/// 在 Finder 中选中配置文件；文件尚未生成时则打开其父目录。
#[cfg(target_os = "macos")]
pub fn reveal(path: &Path) -> std::io::Result<()> {
    let mut cmd = std::process::Command::new("open");
    if path.exists() {
        cmd.arg("-R").arg(path);
    } else if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
        cmd.arg(parent);
    } else {
        cmd.arg(path);
    }
    cmd.spawn().map(|_| ())
}

#[cfg(not(target_os = "macos"))]
pub fn pick_import(_title: &str, _prompt: &str) -> Receiver<Option<PathBuf>> {
    let (tx, rx) = mpsc::channel();
    let _ = tx.send(None);
    rx
}

#[cfg(not(target_os = "macos"))]
pub fn pick_export(
    _title: &str,
    _prompt: &str,
    _default_name: &str,
) -> Receiver<Option<PathBuf>> {
    let (tx, rx) = mpsc::channel();
    let _ = tx.send(None);
    rx
}

#[cfg(not(target_os = "macos"))]
pub fn reveal(_path: &Path) -> std::io::Result<()> {
    Ok(())
}
