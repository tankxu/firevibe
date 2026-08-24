//! 极简 i18n。只覆盖界面实际用到的字符串。
use firevibe_core::config::Lang;

macro_rules! t {
    ($lang:expr, $zh:expr, $en:expr) => {
        match $lang {
            Lang::Zh => $zh,
            Lang::En => $en,
        }
    };
}

pub struct L(pub Lang);

impl L {
    // 顶栏
    pub fn app_sub(&self) -> &'static str {
        t!(self.0, "Fire TV 遥控器控制台", "Fire TV remote console")
    }
    pub fn settings(&self) -> &'static str { t!(self.0, "设置", "Settings") }
    // 菜单栏（tray）
    pub fn tray_show(&self) -> &'static str { t!(self.0, "显示窗口", "Show Window") }
    pub fn tray_quit(&self) -> &'static str { t!(self.0, "退出", "Quit") }

    // 状态条
    pub fn paired(&self) -> &'static str { t!(self.0, "已配对", "Paired") }
    pub fn unpaired(&self) -> &'static str { t!(self.0, "未配对", "Not paired") }
    pub fn connect(&self) -> &'static str { t!(self.0, "连接", "Connect") }
    pub fn disconnect(&self) -> &'static str { t!(self.0, "断开", "Disconnect") }
    pub fn loopback_ready(&self) -> &'static str {
        t!(self.0, "虚拟声卡 · 已就绪", "Virtual device · ready")
    }
    pub fn loopback_missing(&self) -> &'static str {
        t!(self.0, "虚拟声卡 · 未安装", "Virtual device · not installed")
    }
    pub fn loopback_checking(&self) -> &'static str {
        t!(self.0, "虚拟声卡 · 检测中", "Virtual device · checking")
    }
    pub fn system_input(&self) -> &'static str {
        t!(self.0, "系统输入", "System input")
    }
    pub fn install(&self) -> &'static str { t!(self.0, "安装", "Install") }

    // 遥控器
    pub fn remote_hint(&self) -> &'static str {
        t!(
            self.0,
            "点击按键即执行它配的操作\n实体遥控器按下时这里同步高亮",
            "Click a key to run its action.\nPhysical presses light up here."
        )
    }

    // 方案
    pub fn profile(&self) -> &'static str { t!(self.0, "方案", "PROFILE") }
    pub fn new_profile(&self) -> &'static str { t!(self.0, "新建方案", "New profile") }
    pub fn profile_meta(&self, keys: usize, profiles: usize) -> String {
        t!(
            self.0,
            format!("{keys} 个按键已配置 · 共 {profiles} 套方案"),
            format!("{keys} keys configured · {profiles} profiles")
        )
    }

    // 操作卡
    pub fn actions(&self) -> &'static str { t!(self.0, "自定义操作", "CUSTOM ACTIONS") }
    pub fn add_key(&self) -> &'static str { t!(self.0, "添加按键", "Add key") }
    pub fn add_key_hint(&self) -> &'static str {
        t!(self.0, "选一个还没配过的按键", "Pick a key that has no action yet")
    }
    pub fn short_press(&self) -> &'static str { t!(self.0, "短按", "Tap") }
    pub fn long_press(&self) -> &'static str { t!(self.0, "长按", "Hold") }
    pub fn unset(&self) -> &'static str { t!(self.0, "未设置", "Not set") }
    pub fn test(&self) -> &'static str { t!(self.0, "测试", "Test") }
    pub fn edit(&self) -> &'static str { t!(self.0, "编辑", "Edit") }
    pub fn set(&self) -> &'static str { t!(self.0, "设置", "Set") }
    pub fn disable_key(&self) -> &'static str { t!(self.0, "禁用此按键", "Disable this key") }
    pub fn enable_key(&self) -> &'static str { t!(self.0, "启用此按键", "Enable this key") }
    pub fn remove(&self) -> &'static str { t!(self.0, "移除", "Remove") }
    pub fn disabled_tag(&self) -> &'static str { t!(self.0, "已禁用", "Disabled") }

    // 编辑弹窗
    pub fn edit_action(&self) -> &'static str { t!(self.0, "编辑操作", "Edit action") }
    pub fn action_type(&self) -> &'static str { t!(self.0, "动作类型", "ACTION TYPE") }
    pub fn presets(&self) -> &'static str { t!(self.0, "预设", "PRESETS") }
    pub fn modifiers(&self) -> &'static str { t!(self.0, "修饰键", "MODIFIERS") }
    pub fn key_name(&self) -> &'static str { t!(self.0, "按键", "KEY") }
    pub fn hotkey(&self) -> &'static str { t!(self.0, "快捷键", "HOTKEY") }
    pub fn applescript_code(&self) -> &'static str {
        t!(self.0, "AppleScript 代码", "APPLESCRIPT CODE")
    }
    pub fn shell_cmd(&self) -> &'static str { t!(self.0, "命令", "COMMAND") }
    pub fn app_target(&self) -> &'static str {
        t!(self.0, "应用（bundle id / 名称 / 路径）", "APP (bundle id / name / path)")
    }
    pub fn text_arg(&self) -> &'static str { t!(self.0, "文字", "TEXT") }
    pub fn test_once(&self) -> &'static str { t!(self.0, "测试一次", "Test once") }
    pub fn cancel(&self) -> &'static str { t!(self.0, "取消", "Cancel") }
    pub fn save(&self) -> &'static str { t!(self.0, "保存", "Save") }

    // 设置页
    pub fn general(&self) -> &'static str { t!(self.0, "通用", "GENERAL") }
    pub fn about(&self) -> &'static str { t!(self.0, "关于", "ABOUT") }
    pub fn launch_at_login(&self) -> &'static str { t!(self.0, "开机启动", "Launch at login") }
    pub fn launch_hint(&self) -> &'static str {
        t!(self.0, "登录时自动在后台启动", "Start in the background at login")
    }
    pub fn language(&self) -> &'static str { t!(self.0, "语言", "Language") }
    pub fn long_ms(&self) -> &'static str { t!(self.0, "长按判定阈值", "Hold threshold") }
    pub fn long_ms_hint(&self) -> &'static str {
        t!(self.0, "按住超过这个时间算长按", "Held longer than this counts as a hold")
    }
    pub fn check_update(&self) -> &'static str { t!(self.0, "检查更新", "Check for updates") }
    pub fn do_update(&self) -> &'static str { t!(self.0, "更新", "Update") }
    pub fn up_to_date(&self) -> &'static str { t!(self.0, "已是最新", "Up to date") }
    pub fn has_update(&self) -> &'static str { t!(self.0, "有新版本", "Update available") }
    pub fn checking(&self) -> &'static str { t!(self.0, "检查中…", "Checking…") }
    pub fn no_endpoint(&self) -> &'static str {
        t!(self.0, "未配置更新源", "No update source configured")
    }
    pub fn not_checked(&self) -> &'static str { t!(self.0, "未检查", "Not checked") }
}
