//! 极简 i18n。只覆盖界面实际用到的字符串。
use firevibe_core::config::Lang;
use firevibe_core::layout::Slot;

macro_rules! t {
    ($lang:expr, $zh:expr, $en:expr) => {
        match $lang {
            Lang::Zh => $zh,
            Lang::En => $en,
        }
    };
}

#[derive(Clone, Copy)]
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
    // 使用统计
    pub fn stats_title(&self) -> &'static str { t!(self.0, "使用统计", "Usage") }
    pub fn stats_empty(&self) -> &'static str { t!(self.0, "还没有使用记录 —— 按几下遥控器就有了", "No usage yet — press a few keys to get started") }
    pub fn stats_overview(&self) -> &'static str { t!(self.0, "总览", "OVERVIEW") }
    pub fn stats_total(&self) -> String { t!(self.0, "总触发", "Total presses").into() }
    pub fn stats_today(&self) -> String { t!(self.0, "今天", "Today").into() }
    pub fn stats_active_days(&self) -> String { t!(self.0, "活跃天数", "Active days").into() }
    pub fn stats_since(&self) -> String { t!(self.0, "起始", "Since").into() }
    pub fn stats_by_key(&self) -> &'static str { t!(self.0, "按键排行", "BY KEY") }
    pub fn stats_by_action(&self) -> &'static str { t!(self.0, "动作类型", "BY ACTION") }
    pub fn stats_voice(&self) -> &'static str { t!(self.0, "语音", "VOICE") }
    pub fn stats_voice_count(&self) -> String { t!(self.0, "语音次数", "Voice uses").into() }
    pub fn stats_voice_dur(&self) -> String { t!(self.0, "累计时长", "Total time").into() }
    pub fn stats_battery(&self) -> String { t!(self.0, "当前电量", "Battery").into() }
    /// ActionType 的 Debug 名 -> 界面显示
    pub fn action_type_name(&self, dbg: &str) -> String {
        match dbg {
            "Key" => t!(self.0, "映射按键", "Key"),
            "Text" => t!(self.0, "输入文字", "Type text"),
            "OpenApp" => t!(self.0, "打开应用", "Open app"),
            "AppleScript" => "AppleScript",
            "Shell" => t!(self.0, "执行命令", "Shell"),
            "Http" => t!(self.0, "HTTP 请求", "HTTP request"),
            "VoicePtt" => t!(self.0, "按住说话", "Push to talk"),
            "VoiceToggle" => t!(self.0, "开关说话", "Toggle talk"),
            "VoiceDictate" => t!(self.0, "语音转文字", "Dictation"),
            "VoiceHotkey" => t!(self.0, "第三方语音输入", "Third-party voice"),
            "Record" => t!(self.0, "录音", "Record"),
            other => other,
        }.to_string()
    }
    pub fn menu_quit(&self) -> &'static str { t!(self.0, "退出 FireVibe", "Quit FireVibe") }
    // 悬浮电平条 HUD
    pub fn hud_dictating(&self) -> &'static str { t!(self.0, "松手出字", "Release to type") }
    pub fn hud_mic_on(&self) -> &'static str { t!(self.0, "麦克风已开", "Mic on") }

    // 按键名（core 的 Slot::label 是中文，这里做界面侧本地化，不动 core）
    pub fn slot_label(&self, s: Slot) -> &'static str {
        match s {
            Slot::Power => t!(self.0, "电源", "Power"),
            Slot::Mic => t!(self.0, "麦克风", "Mic"),
            Slot::Up => t!(self.0, "上", "Up"),
            Slot::Left => t!(self.0, "左", "Left"),
            Slot::Ok => "OK",
            Slot::Right => t!(self.0, "右", "Right"),
            Slot::Down => t!(self.0, "下", "Down"),
            Slot::Back => t!(self.0, "返回", "Back"),
            Slot::Home => t!(self.0, "主页", "Home"),
            Slot::Menu => t!(self.0, "菜单", "Menu"),
            Slot::Rewind => t!(self.0, "快退", "Rewind"),
            Slot::Play => t!(self.0, "播放/暂停", "Play/Pause"),
            Slot::Forward => t!(self.0, "快进", "Forward"),
            Slot::Mute => t!(self.0, "静音", "Mute"),
            Slot::VolUp => t!(self.0, "音量+", "Vol +"),
            Slot::VolDown => t!(self.0, "音量−", "Vol −"),
            Slot::Tv => "TV",
            Slot::App1 => "Prime Video",
            Slot::App2 => "Netflix",
            Slot::App3 => "Disney+",
            Slot::App4 => "Hulu",
        }
    }
    // 卡片标题：四个 App 键按机身印字，其余用 slot_label
    pub fn card_title(&self, s: Slot) -> &'static str {
        match s {
            Slot::App1 => t!(self.0, "Prime Video 键", "Prime Video"),
            Slot::App2 => t!(self.0, "NETFLIX 键", "NETFLIX"),
            Slot::App3 => t!(self.0, "Disney+ 键", "Disney+"),
            Slot::App4 => t!(self.0, "hulu 键", "hulu"),
            other => self.slot_label(other),
        }
    }
    // 组合键里的按键写法
    pub fn key_space(&self) -> &'static str { t!(self.0, "空格", "Space") }
    pub fn key_none(&self) -> &'static str { t!(self.0, "未选", "None") }

    // 启动/语音链路错误 toast
    pub fn toast_block_failed(&self, e: &str) -> String {
        if e.contains("EVENT_TAP_FAILED") {
            return t!(self.0,
                "无法屏蔽系统默认行为 —— 多半是缺「辅助功能」权限".to_string(),
                "Couldn't suppress system default keys — likely missing Accessibility permission".to_string());
        }
        t!(self.0, format!("屏蔽系统默认行为失败: {e}"), format!("Couldn't block the system default action: {e}"))
    }
    pub fn toast_voice_start_failed(&self, e: &str) -> String { t!(self.0, format!("语音链路启动失败: {e}"), format!("Voice pipeline failed to start: {e}")) }
    // 更新状态里的版本行
    pub fn update_available_ver(&self, cur: &str, new: &str) -> String { t!(self.0, format!("{cur} → {new} 可更新"), format!("{cur} → {new} available")) }

    // 空方案占位
    pub fn empty_profile_title(&self) -> &'static str { t!(self.0, "这套方案还没有自定义按键", "This profile has no custom keys yet") }
    pub fn empty_profile_hint(&self) -> &'static str { t!(self.0, "遥控器上的键保持系统原本的行为。点右上角「添加按键」挑一颗来配。", "Remote keys keep their system behavior. Tap Add key in the top-right to configure one.") }
    pub fn toast_dl_opened(&self) -> &'static str { t!(self.0, "已打开 BlackHole 下载页", "Opened the BlackHole download page") }

    // cards::describe —— 每个动作在卡片上的一句话描述
    pub fn dsc_key(&self, combo: &str) -> String { t!(self.0, format!("映射按键 · {combo}"), format!("Key · {combo}")) }
    pub fn dsc_text(&self) -> &'static str { t!(self.0, "输入文字", "Type text") }
    pub fn dsc_open(&self, app: &str) -> String { t!(self.0, format!("打开 {app}"), format!("Open {app}")) }
    pub fn dsc_applescript(&self, name: &str) -> String { t!(self.0, format!("AppleScript · {name}"), format!("AppleScript · {name}")) }
    pub fn dsc_custom(&self) -> &'static str { t!(self.0, "自定义", "Custom") }
    pub fn dsc_shell(&self) -> &'static str { t!(self.0, "执行命令", "Run command") }
    pub fn dsc_http(&self, m: &str) -> String { t!(self.0, format!("HTTP {m}"), format!("HTTP {m}")) }
    pub fn dsc_voice_toggle(&self) -> &'static str { t!(self.0, "开始 / 停止说话", "Start / stop talking") }
    pub fn dsc_voice_toggle_hint(&self) -> &'static str { t!(self.0, "点一下开始，再点一下停止", "Tap to start, tap again to stop") }
    pub fn dsc_ptt(&self) -> &'static str { t!(self.0, "按住说话", "Hold to talk") }
    pub fn dsc_ptt_hint(&self) -> &'static str { t!(self.0, "按住送流，松手停止", "Streams while held, stops on release") }
    pub fn dsc_record(&self) -> &'static str { t!(self.0, "按住录音", "Hold to record") }
    pub fn dsc_record_hint(&self) -> &'static str { t!(self.0, "松手保存到「下载」", "Saves to Downloads on release") }
    pub fn dsc_dictate(&self) -> &'static str { t!(self.0, "语音转文字", "Speech to text") }
    pub fn dsc_dictate_hold(&self) -> &'static str { t!(self.0, "按住说话，松手识别并打字", "Hold to talk, release to transcribe and type") }
    pub fn dsc_dictate_tap(&self) -> &'static str { t!(self.0, "点一下开始，再点一下结束并识别", "Tap to start, tap again to end and transcribe") }
    pub fn dsc_hotkey(&self, combo: &str) -> String { t!(self.0, format!("第三方语音输入 · {combo}"), format!("Third-party voice input · {combo}")) }
    pub fn dsc_mode_hold(&self) -> &'static str { t!(self.0, "按住期间保持按下", "Held down while pressed") }
    pub fn dsc_mode_double(&self) -> &'static str { t!(self.0, "双击", "Double tap") }
    pub fn dsc_mode_tap(&self) -> &'static str { t!(self.0, "敲一下", "Single tap") }
    pub fn cli_tool(&self) -> &'static str { t!(self.0, "命令行工具 firectl", "Command-line tool (firectl)") }
    pub fn cli_tool_sub(&self) -> &'static str { t!(self.0, "配新遥控器 / 排障用", "For setting up a new remote / diagnostics") }
    pub fn download(&self) -> &'static str { t!(self.0, "下载", "Download") }

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
    pub fn configuration(&self) -> &'static str { t!(self.0, "配置文件", "CONFIGURATION") }
    pub fn about(&self) -> &'static str { t!(self.0, "关于", "ABOUT") }
    pub fn config_location(&self) -> &'static str { t!(self.0, "配置文件位置", "Configuration file") }
    pub fn config_manage(&self) -> &'static str { t!(self.0, "导入与导出", "Import and export") }
    pub fn config_manage_hint(&self) -> &'static str {
        t!(self.0, "导入前会把当前配置备份为 config.backup.json", "The current configuration is backed up as config.backup.json before importing")
    }
    pub fn reload_config(&self) -> &'static str { t!(self.0, "重新加载", "Reload") }
    pub fn reveal_finder(&self) -> &'static str { t!(self.0, "在 Finder 中显示", "Show in Finder") }
    pub fn import_config(&self) -> &'static str { t!(self.0, "导入", "Import") }
    pub fn export_config(&self) -> &'static str { t!(self.0, "导出", "Export") }
    pub fn import_config_title(&self) -> &'static str { t!(self.0, "导入 FireVibe 配置", "Import FireVibe Configuration") }
    pub fn export_config_title(&self) -> &'static str { t!(self.0, "导出 FireVibe 配置", "Export FireVibe Configuration") }
    pub fn toast_config_reloaded(&self) -> &'static str { t!(self.0, "配置已重新加载并生效", "Configuration reloaded and applied") }
    pub fn toast_config_reload_failed(&self, e: &str) -> String {
        t!(self.0, format!("重新加载失败：{e}"), format!("Reload failed: {e}"))
    }
    pub fn toast_config_imported(&self) -> &'static str { t!(self.0, "配置已导入并生效", "Configuration imported and applied") }
    pub fn toast_config_exported(&self) -> &'static str { t!(self.0, "配置已导出", "Configuration exported") }
    pub fn toast_config_import_failed(&self, e: &str) -> String {
        t!(self.0, format!("导入失败：{e}"), format!("Import failed: {e}"))
    }
    pub fn toast_config_export_failed(&self, e: &str) -> String {
        t!(self.0, format!("导出失败：{e}"), format!("Export failed: {e}"))
    }
    pub fn toast_config_reveal_failed(&self, e: &str) -> String {
        t!(self.0, format!("无法打开配置位置：{e}"), format!("Couldn't reveal the configuration file: {e}"))
    }
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

    // 设置页 · 语音/通用
    pub fn auto_switch_input(&self) -> &'static str { t!(self.0, "说话时自动切输入设备", "Switch input while talking") }
    pub fn auto_switch_hint(&self) -> &'static str {
        t!(self.0, "按下说话键把系统默认输入切到虚拟声卡，说完切回原来的（实测 3~13ms）",
           "While the mic key is held, switch the system input to the virtual device, then switch back.")
    }
    pub fn show_hud(&self) -> &'static str { t!(self.0, "说话时显示电平条", "Show level bar while talking") }
    pub fn show_hud_hint(&self) -> &'static str {
        t!(self.0, "屏幕底部浮出一条电平，让你知道确实在收音",
           "A floating meter at the bottom of the screen, so you know it's capturing.")
    }
    pub fn voice_to_text(&self) -> &'static str { t!(self.0, "语音转文字", "SPEECH TO TEXT") }
    pub fn stt_perm(&self) -> &'static str { t!(self.0, "语音识别权限", "Speech recognition permission") }
    pub fn stt_perm_ok(&self) -> &'static str {
        t!(self.0, "已授权，可以用「语音转文字」动作", "Granted — the Dictate action is available")
    }
    pub fn stt_perm_no(&self) -> &'static str {
        t!(self.0, "系统自带的离线识别需要一次授权", "The built-in offline recognizer needs a one-time grant")
    }
    pub fn granted(&self) -> &'static str { t!(self.0, "已授权", "Granted") }
    pub fn request_perm(&self) -> &'static str { t!(self.0, "请求授权", "Grant access") }
    pub fn stt_lang(&self) -> &'static str { t!(self.0, "识别语言", "Recognition language") }
    pub fn stt_lang_hint(&self) -> &'static str {
        t!(self.0, "自带语音转文字识别成哪种语言", "Which language the built-in speech-to-text recognizes")
    }
    pub fn stt_enter(&self) -> &'static str { t!(self.0, "识别后自动回车", "Press Return after recognizing") }
    pub fn stt_enter_hint(&self) -> &'static str {
        t!(self.0, "在 agent 里就是说完直接发出去", "In an agent, this sends the message as soon as you finish speaking")
    }
    pub fn check_failed(&self) -> &'static str { t!(self.0, "检查失败", "Check failed") }
    pub fn toast_login_added(&self) -> &'static str { t!(self.0, "已加入登录启动项", "Added to login items") }
    pub fn toast_login_removed(&self) -> &'static str { t!(self.0, "已移出登录启动项", "Removed from login items") }
    pub fn toast_stt_prompt(&self) -> &'static str { t!(self.0, "已弹出系统授权框", "System prompt shown") }
    pub fn toast_no_download_url(&self) -> &'static str { t!(self.0, "更新里没给下载地址", "No download URL in the release") }

    // 首次引导
    pub fn onb_title(&self) -> &'static str { t!(self.0, "欢迎使用 FireVibe", "Welcome to FireVibe") }
    pub fn onb_subtitle(&self) -> &'static str {
        t!(self.0, "把 Fire TV 语音遥控器变成 Mac 的遥控器 + 麦克风。开始前配好下面几项：",
           "Turn a Fire TV voice remote into a remote + mic for your Mac. Set up these first:")
    }
    pub fn onb_pair(&self) -> &'static str { t!(self.0, "配对遥控器", "Pair the remote") }
    pub fn onb_pair_desc(&self) -> &'static str {
        t!(self.0, "系统设置 › 蓝牙 里把遥控器和 Mac 配上（长按遥控器 home 键进配对）。",
           "Pair the remote in System Settings › Bluetooth (hold the remote's home key to enter pairing).")
    }
    pub fn onb_im(&self) -> &'static str { t!(self.0, "开启「输入监控」权限", "Enable Input Monitoring") }
    pub fn onb_im_desc(&self) -> &'static str {
        t!(self.0, "系统设置 › 隐私与安全性 › 输入监控 → 勾上 FireVibe。读按键和麦克风都要它。",
           "System Settings › Privacy & Security › Input Monitoring → enable FireVibe. Needed for both keys and mic.")
    }
    pub fn onb_card(&self) -> &'static str { t!(self.0, "安装虚拟声卡「FireVibe Mic」", "Install the \"FireVibe Mic\" virtual device") }
    pub fn onb_card_desc(&self) -> &'static str {
        t!(self.0, "语音输入靠它把遥控器麦克风喂给第三方语音输入工具。点安装会弹系统授权框。",
           "Voice input uses it to feed the remote's mic to third-party voice input tools. Install prompts for admin access.")
    }
    pub fn onb_bt(&self) -> &'static str { t!(self.0, "允许蓝牙（可选，用于显示电量）", "Allow Bluetooth (optional, for battery level)") }
    pub fn onb_bt_desc(&self) -> &'static str {
        t!(self.0, "首次读电量会弹「FireVibe 想使用蓝牙」，点允许即可；不需要电量可跳过。",
           "The first battery read prompts \"FireVibe wants to use Bluetooth\" — just allow it, or skip if you don't need battery.")
    }
    pub fn onb_ready(&self) -> &'static str { t!(self.0, "已就绪", "Ready") }
    pub fn onb_open_bt(&self) -> &'static str { t!(self.0, "打开蓝牙", "Open Bluetooth") }
    pub fn onb_open_settings(&self) -> &'static str { t!(self.0, "打开设置", "Open Settings") }
    pub fn onb_footer(&self) -> &'static str { t!(self.0, "这些随时能在「设置」里再弄", "You can do these later in Settings") }
    pub fn onb_start(&self) -> &'static str { t!(self.0, "开始使用", "Get started") }
    pub fn toast_card_installed(&self) -> &'static str { t!(self.0, "声卡装好了", "Virtual device installed") }

    // 安装声卡弹窗
    pub fn install_title(&self, dev: &str) -> String {
        t!(self.0, format!("安装虚拟声卡「{dev}」"), format!("Install the \"{dev}\" virtual device"))
    }
    pub fn install_body(&self) -> &'static str {
        t!(self.0,
           "虚拟声卡是一块只存在于软件里的声卡。遥控器麦克风的音频写进它，第三方语音输入工具就能把它当麦克风来听。\n\n它要装到系统的音频插件目录，所以需要管理员权限 —— 接下来会弹一个系统自带的授权框。",
           "A virtual audio device exists only in software. The remote's mic audio is written into it, so third-party voice input tools can listen to it as a microphone.\n\nIt installs into the system audio plug-in folder, which needs admin access — a system prompt will appear next.")
    }
    pub fn install_continue(&self) -> &'static str { t!(self.0, "继续安装", "Continue") }
    pub fn toast_installed_hint(&self, dev: &str) -> String {
        t!(self.0, format!("装好了。在语音工具里把麦克风选成 {dev}"),
           format!("Installed. Select {dev} as the mic in your voice tool."))
    }
    pub fn toast_install_failed(&self, e: &str) -> String {
        t!(self.0, format!("安装失败：{e}"), format!("Install failed: {e}"))
    }

    // 语音测试面板
    pub fn vt_title(&self) -> &'static str { t!(self.0, "测试语音输入", "Test voice input") }
    pub fn vt_hint(&self) -> &'static str { t!(self.0, "按住下面的按钮，对着遥控器说话", "Hold the button below and speak into the remote") }
    pub fn vt_level(&self) -> &'static str { t!(self.0, "电平", "LEVEL") }
    pub fn vt_level_line(&self, lvl: f32, frames: u64, on: bool) -> String {
        let mic = if on { t!(self.0, "开", "on") } else { t!(self.0, "关", "off") };
        t!(self.0, format!("电平 {lvl:.4}   本次收到 {frames} 帧   麦克风 {mic}"),
           format!("Level {lvl:.4}   {frames} frames   mic {mic}"))
    }
    pub fn vt_default_input(&self) -> &'static str { t!(self.0, "系统默认输入", "SYSTEM DEFAULT INPUT") }
    pub fn vt_caption_on(&self) -> &'static str {
        t!(self.0, "跟随系统默认输入的第三方语音工具现在能听到遥控器",
           "Third-party tools that follow the system default input can now hear the remote")
    }
    pub fn vt_caption_off(&self) -> &'static str {
        t!(self.0, "按住测试时会临时切到虚拟声卡，松开还原",
           "Holding to test switches to the virtual device temporarily, then switches back")
    }
    pub fn vt_recording(&self) -> &'static str { t!(self.0, "正在收音…（松开结束）", "Listening… (release to stop)") }
    pub fn vt_hold_talk(&self) -> &'static str { t!(self.0, "按住说话", "Hold to talk") }
    pub fn vt_dictating(&self) -> &'static str { t!(self.0, "正在听写…（松开出字）", "Dictating… (release for text)") }
    pub fn vt_hold_dictate(&self) -> &'static str { t!(self.0, "按住听写 · 转成文字", "Hold to dictate · to text") }
    pub fn vt_result(&self) -> &'static str { t!(self.0, "识别结果", "RESULT") }
    pub fn toast_card_not_ready(&self) -> &'static str { t!(self.0, "虚拟声卡还没就绪", "Virtual device isn't ready yet") }
    pub fn toast_link_not_ready(&self) -> &'static str { t!(self.0, "语音链路还没建起来，稍等一下", "Voice link isn't ready yet, hold on") }

    // 错误条（HID / 输入监控）
    pub fn hid_no_perm(&self) -> &'static str { t!(self.0, "遥控器打不开：缺「输入监控」权限", "Can't open the remote: missing Input Monitoring") }
    pub fn hid_not_connected(&self) -> &'static str { t!(self.0, "遥控器没连上", "Remote isn't connected") }
    pub fn hid_not_found_hint(&self) -> &'static str {
        t!(self.0,
           "没找到匹配的遥控器。按一下遥控器任意键唤醒它；换了新遥控器就点「重新配对」按型号重新适配。",
           "No matching remote found. Press any key on it to wake it; if you switched remotes, tap “Re-pair” to set up the new one.")
    }
    pub fn re_pair(&self) -> &'static str { t!(self.0, "配对新遥控器", "Pair new remote") }
    pub fn pair_title(&self) -> &'static str { t!(self.0, "配对新遥控器", "Pair a new remote") }
    pub fn pair_hint(&self) -> &'static str { t!(self.0, "先在 系统设置 › 蓝牙 里连上遥控器（按一下它任意键唤醒），再从下面选中它。", "Connect the remote in System Settings › Bluetooth first (press any key to wake it), then pick it below.") }
    pub fn pair_scanning(&self) -> &'static str { t!(self.0, "正在扫描设备…", "Scanning devices…") }
    pub fn pair_none(&self) -> &'static str { t!(self.0, "没扫到设备。确认遥控器已在蓝牙里连上、按一下它唤醒，再「重新扫描」。", "No devices found. Make sure the remote is connected in Bluetooth and awake, then Rescan.") }
    pub fn pair_likely(&self) -> &'static str { t!(self.0, "像遥控器", "likely remote") }
    pub fn pair_current(&self) -> &'static str { t!(self.0, "当前", "current") }
    pub fn pair_rescan(&self) -> &'static str { t!(self.0, "重新扫描", "Rescan") }
    // ── 红外遥控 ──
    pub fn dsc_ir(&self) -> &'static str { t!(self.0, "红外遥控", "IR remote") }
    pub fn ir_code_label(&self) -> &'static str { t!(self.0, "红外码", "IR code") }
    pub fn ir_help(&self) -> &'static str {
        t!(self.0,
           "遥控器自带红外发射管。两种码都能直接粘，自动识别：Pronto hex（0000 006D …，网上码库最常见），或抓码工具直出的原始数组（9000, 4500, …，默认按 38 kHz）。",
           "The remote has its own IR emitter. Paste either kind of code — auto-detected: Pronto hex (0000 006D …, the most widely published), or a raw timing array straight from a capture tool (9000, 4500, … — assumed 38 kHz).")
    }
    pub fn ir_limits(&self) -> &'static str {
        t!(self.0,
           "限制来自遥控器固件：最多 2 段，每个时长 ≤ 32767 µs。空调那种多帧长码放不下。",
           "Limits come from the remote's firmware: at most 2 sequences, each duration ≤ 32767 µs. Multi-frame A/C codes won't fit.")
    }
    pub fn ir_not_wired(&self) -> &'static str {
        t!(self.0,
           "发射通道还没接通 —— 现在只校验码对不对",
           "Transmit path isn't wired up yet — this only validates the code for now")
    }

    // ── PTT 遥控器提醒 ──
    pub fn ptt_hint(&self) -> &'static str {
        t!(self.0,
           "这台遥控器只有按住麦克风键时才出声，语音要配在「长按」里",
           "This remote only streams while the mic key is held — configure voice under Long press")
    }
    pub fn ptt_fix(&self) -> &'static str { t!(self.0, "移到长按", "Move to long press") }
    pub fn ptt_fixed(&self) -> &'static str { t!(self.0, "已移到长按", "Moved to long press") }
    pub fn ptt_other_key_note(&self) -> &'static str {
        t!(self.0,
           "这台遥控器只有按住麦克风键时才出声，其它按键跟音频没有关系 —— 所以这里不提供语音动作。语音请配在麦克风键的「长按」里。",
           "This remote only streams while its mic key is held; other keys have nothing to do with audio, so voice actions aren't offered here. Configure voice under the mic key's Long press.")
    }
    pub fn ptt_vs_hotkey_note(&self) -> &'static str {
        t!(self.0,
           "这个动作只把音频送进虚拟声卡，不会发任何快捷键。如果还需要同时按下热键去唤起语音工具，请改用「第三方语音输入」。",
           "This only feeds audio into the virtual mic — it sends no keystroke. If you also need a hotkey pressed to wake your voice tool, use “Third-party voice input” instead.")
    }
    pub fn ptt_short_note(&self) -> &'static str {
        t!(self.0,
           "这台遥控器的麦克风只支持按住 —— 点一下松手就没有音频了，所以短按这里不提供语音动作。请到「长按」里配置。",
           "This remote's mic only works while held — a tap produces no audio, so voice actions aren't offered here. Configure them under Long press.")
    }

    pub fn pair_ok(&self) -> &'static str { t!(self.0, "已连上新遥控器", "Connected to the new remote") }
    pub fn pair_saved(&self) -> &'static str { t!(self.0, "已保存设备，按一下遥控器唤醒它就会连上", "Device saved — press a key on the remote to connect") }
    pub fn repair_toast(&self) -> &'static str {
        t!(self.0,
           "在终端跑：firectl --probe-all（跟着提示走，会把新遥控器写进配置）",
           "Run in Terminal: firectl --probe-all (follow the prompts to set up the new remote)")
    }
    pub fn hid_open_failed(&self) -> &'static str { t!(self.0, "遥控器打不开", "Can't open the remote") }
    pub fn hid_perm_hint(&self) -> &'static str {
        t!(self.0,
           "到 系统设置 › 隐私与安全性 › 输入监控 勾上本应用，然后完全退出重开；已经勾着还报这个，就点「重置授权」再勾一次",
           "In System Settings › Privacy & Security › Input Monitoring, enable this app, then fully quit and reopen. If it's already on and you still see this, hit Reset access and enable it again.")
    }
    pub fn retry(&self) -> &'static str { t!(self.0, "重试", "Retry") }
    pub fn open_settings(&self) -> &'static str { t!(self.0, "打开设置", "Open Settings") }
    pub fn reset_auth(&self) -> &'static str { t!(self.0, "重置授权", "Reset access") }
    pub fn toast_connected(&self) -> &'static str { t!(self.0, "已连上", "Connected") }
    pub fn toast_reset_done(&self) -> &'static str { t!(self.0, "已重置，去系统设置里重新勾一次，然后完全退出重开", "Reset. Re-enable it in System Settings, then fully quit and reopen.") }
    pub fn toast_reset_failed(&self) -> &'static str { t!(self.0, "重置失败，手动到系统设置里取消勾选再勾上", "Reset failed — toggle it off and on manually in System Settings") }

    // 状态区 STT 警示
    pub fn stt_unavailable(&self) -> &'static str { t!(self.0, "语音转文字还不能用：缺「语音识别」权限", "Speech to text isn't available: missing Speech Recognition") }
    // core 的 auth_status() 返回中文，这里映射成当前语言
    pub fn stt_status_label(&self, st: &str) -> &'static str {
        match st {
            "已授权" => t!(self.0, "已授权", "Authorized"),
            "被拒绝" => t!(self.0, "被拒绝", "Denied"),
            "受限" => t!(self.0, "受限", "Restricted"),
            "未决定" => t!(self.0, "未决定", "Not determined"),
            _ => t!(self.0, "这个平台没有自带语音识别", "No built-in speech recognition on this platform"),
        }
    }
    pub fn stt_ask_hint(&self, st: &str) -> String {
        let st = self.stt_status_label(st);
        t!(self.0, format!("{st} · 点右侧请求授权，弹框选「允许」"),
           format!("{st} · tap Grant on the right and choose Allow"))
    }
    pub fn toast_requested(&self) -> &'static str { t!(self.0, "已请求，系统弹框里选「允许」", "Requested — choose Allow in the system prompt") }

    // 电量
    pub fn battery_needs_bt(&self) -> &'static str { t!(self.0, "电量需蓝牙权限", "Battery needs Bluetooth") }
    pub fn grant_access(&self) -> &'static str { t!(self.0, "去授权", "Grant") }

    // 录音
    pub fn recording_time(&self, mm: u64, ss: u64) -> String {
        t!(self.0, format!("录音中 {mm:02}:{ss:02}"), format!("Recording {mm:02}:{ss:02}"))
    }
    pub fn recording_stop_hint(&self) -> &'static str { t!(self.0, "松手保存到「下载」", "Release to save to Downloads") }

    // 方案改名
    pub fn rename(&self) -> &'static str { t!(self.0, "重命名", "Rename") }
    pub fn rename_title(&self) -> &'static str { t!(self.0, "方案重命名", "Rename profile") }
    pub fn toast_name_empty(&self) -> &'static str { t!(self.0, "名字不能为空", "Name can't be empty") }

    // 编辑弹窗 · 硬件修饰键 / 触发方式 / HTTP / 听写
    pub fn trigger_mode(&self) -> &'static str { t!(self.0, "触发方式", "Trigger") }
    pub fn single_tap(&self) -> &'static str { t!(self.0, "单击一下", "Single tap") }
    pub fn double_tap(&self) -> &'static str { t!(self.0, "双击", "Double tap") }
    pub fn hotkey_hold_hint(&self) -> &'static str {
        t!(self.0,
           "长按遥控器时按住这个快捷键不放，松手才松开 —— 对应第三方语音输入工具的「按住说话」。注意 Fn 是硬件级的，合成不出去；单独的修饰键能发出事件，但改不了系统全局修饰位，那个工具认不认只能实测，想稳就用普通组合键。",
           "On a long press, this hotkey is held down until you release — matching a third-party voice tool's push-to-talk. Note: Fn is hardware-level and can't be synthesized; a lone modifier fires an event but doesn't change the global modifier state, so whether a tool accepts it can only be tested. For reliability, use a normal key combo.")
    }
    pub fn hotkey_tap_hint(&self) -> &'static str {
        t!(self.0,
           "短按遥控器时敲一下这个快捷键 —— 对应第三方语音输入工具的「按一下开始、再按一下结束」。注意 Fn 是硬件级的，合成不出去；单独的修饰键能发出事件，但改不了系统全局修饰位，那个工具认不认只能实测，想稳就用普通组合键。",
           "On a short press, this hotkey is tapped once — matching a third-party voice tool's tap-to-start, tap-to-stop. Note: Fn is hardware-level and can't be synthesized; a lone modifier fires an event but doesn't change the global modifier state, so whether a tool accepts it can only be tested. For reliability, use a normal key combo.")
    }
    pub fn dictate_hold_hint(&self) -> &'static str {
        t!(self.0,
           "按住遥控器说话，松手后用系统自带的离线识别转成文字，打进当前焦点。不依赖任何第三方工具，也不会动你的系统输入设备。语言和「识别后自动回车」在设置里调。",
           "Hold the remote and speak; on release, the built-in offline recognizer turns it into text typed into the focused field. No third-party tool, and your system input device is left alone. Language and Press-Return-after are in Settings.")
    }
    pub fn dictate_tap_hint(&self) -> &'static str {
        t!(self.0,
           "点一下开始录，再点一下结束并识别，文字打进当前焦点。不依赖任何第三方工具，也不会动你的系统输入设备。语言和「识别后自动回车」在设置里调。",
           "Tap once to start, tap again to stop and recognize; the text is typed into the focused field. No third-party tool, and your system input device is left alone. Language and Press-Return-after are in Settings.")
    }
    pub fn stt_perm_label(&self, st: &str) -> String {
        let st = self.stt_status_label(st);
        t!(self.0, format!("语音识别权限：{st}"), format!("Speech recognition: {st}"))
    }
    pub fn toast_stt_prompt2(&self) -> &'static str { t!(self.0, "已弹出系统授权框，允许之后回来再看", "System prompt shown — come back after allowing it") }
    pub fn http_method(&self) -> &'static str { t!(self.0, "方法", "METHOD") }
    pub fn http_body(&self) -> &'static str { t!(self.0, "请求体", "BODY") }
    pub fn http_retries(&self) -> &'static str { t!(self.0, "重试次数", "RETRIES") }
    pub fn http_timeout(&self) -> &'static str { t!(self.0, "超时 (毫秒)", "TIMEOUT (ms)") }
    pub fn toast_executed(&self) -> &'static str { t!(self.0, "已执行", "Done") }
    pub fn hotkey_recording(&self) -> &'static str { t!(self.0, "按下组合键…（Esc 取消）", "Press a combo… (Esc to cancel)") }
    pub fn hotkey_click_record(&self) -> &'static str { t!(self.0, "点这里录制快捷键", "Click to record a hotkey") }
    pub fn toast_record_window_mode(&self, e: &str) -> String {
        t!(self.0, format!("录制退回窗口模式（{e}）"), format!("Recording fell back to window mode ({e})"))
    }
    pub fn clear(&self) -> &'static str { t!(self.0, "清除", "Clear") }
    pub fn single_modifier_hint(&self) -> &'static str {
        t!(self.0,
           "或者只用一个修饰键 —— 这些改不了系统全局修饰位，能不能驱动那个工具只能实测",
           "Or use a single modifier — these don't change the global modifier state, so whether they drive that tool can only be tested")
    }
    pub fn mod_left(&self) -> &'static str { t!(self.0, "左", "L") }
    pub fn mod_right(&self) -> &'static str { t!(self.0, "右", "R") }

}
