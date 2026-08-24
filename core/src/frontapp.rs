//! 「刚才在哪个 app」—— 听写打字前得知道字该落到谁身上。
//!
//! 打字用的是 CGEvent，谁在前台就打给谁。所以按下麦克风时先记住前台 app，
//! 松手识别完如果前台变了（比如用户顺手点了 FireVibe 的窗口），
//! 先把它切回去再打，不然字就丢进一个没有输入框的窗口里，
//! 表现正好是「识别出来了但没填进去」。

#[cfg(target_os = "macos")]
mod imp {
    use objc2_app_kit::{NSApplicationActivationOptions, NSRunningApplication, NSWorkspace};

    #[derive(Clone, Debug, PartialEq)]
    pub struct FrontApp {
        pub pid: i32,
        pub name: String,
        pub bundle_id: Option<String>,
    }

    pub fn front() -> Option<FrontApp> {
        let app = NSWorkspace::sharedWorkspace().frontmostApplication()?;
        Some(FrontApp {
            pid: app.processIdentifier(),
            name: app
                .localizedName()
                .map(|s| s.to_string())
                .unwrap_or_default(),
            bundle_id: app.bundleIdentifier().map(|s| s.to_string()),
        })
    }

    /// 把某个进程切回前台。返回是否发出去了。
    pub fn activate(pid: i32) -> bool {
        let Some(app) = NSRunningApplication::runningApplicationWithProcessIdentifier(pid) else {
            return false;
        };
        app.activateWithOptions(NSApplicationActivationOptions::empty())
    }

    pub fn self_pid() -> i32 {
        std::process::id() as i32
    }
}

#[cfg(not(target_os = "macos"))]
mod imp {
    #[derive(Clone, Debug, PartialEq)]
    pub struct FrontApp {
        pub pid: i32,
        pub name: String,
        pub bundle_id: Option<String>,
    }
    pub fn front() -> Option<FrontApp> {
        None
    }
    pub fn activate(_pid: i32) -> bool {
        false
    }
    pub fn self_pid() -> i32 {
        std::process::id() as i32
    }
}

pub use imp::{activate, front, self_pid, FrontApp};
