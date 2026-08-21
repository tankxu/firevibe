//! 配置：profile（多套按键方案）+ 语音 + 应用设置。
//!
//! 动作是**按图上位置（Slot）**绑的，不是按 HID usage。
//! 因为四个 App 快捷键的 usage 未知（需要学习），按位置绑才能先把默认动作配好，
//! 等 usage 学到了自动生效。Slot -> usage 的映射单独存在 `slots` 里。

use crate::keys::Key;
use crate::layout::{default_slots, Slot, SlotBinding};
use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

// ---------------- 动作 ----------------

#[derive(Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Debug, Default)]
#[serde(rename_all = "snake_case")]
pub enum ActionType {
    /// 什么都不做（屏蔽这个键）
    #[default]
    None,
    /// 合成一次键盘按键（可带修饰键）
    Key,
    /// 输入一段文字
    Text,
    /// 打开应用（bundle id / 应用名 / 路径）
    OpenApp,
    /// 执行 AppleScript（仅 macOS）
    AppleScript,
    /// 执行 shell 命令
    Shell,
    /// 按住把遥控器麦克风送进虚拟声卡（松手停）
    VoicePtt,
    /// 点一下开始送流，再点一下停止
    VoiceToggle,
    /// 自带语音识别：说话转文字打进当前焦点，不依赖任何第三方工具，
    /// 也不碰系统输入设备。`arg` 存模式，跟着短按/长按走：
    /// `"hold"` = 按住说话（长按），`"tap"` = 点一下开始、再点一下结束（短按）
    VoiceDictate,
    /// 发一个快捷键去触发第三方语音输入工具，由它负责识别并把文字打进当前焦点。
    /// `arg` 存模式，跟着短按/长按走：`"tap"` = 敲一下（短按），
    /// `"hold"` = 按住期间一直按着（长按）
    VoiceHotkey,
}
impl ActionType {
    pub const ALL: [ActionType; 10] = [
        ActionType::None,
        ActionType::Key,
        ActionType::Text,
        ActionType::OpenApp,
        ActionType::AppleScript,
        ActionType::Shell,
        ActionType::VoiceToggle,
        ActionType::VoicePtt,
        ActionType::VoiceHotkey,
        ActionType::VoiceDictate,
    ];
    pub fn label(self) -> &'static str {
        match self {
            ActionType::None => "无",
            ActionType::Key => "映射按键",
            ActionType::Text => "输入文字",
            ActionType::OpenApp => "打开应用",
            ActionType::AppleScript => "AppleScript",
            ActionType::Shell => "执行命令",
            ActionType::VoicePtt => "按住说话",
            ActionType::VoiceToggle => "开始 / 停止说话",
            ActionType::VoiceHotkey => "第三方语音输入",
            ActionType::VoiceDictate => "语音转文字",
        }
    }
    pub fn hint(self) -> &'static str {
        match self {
ActionType::None => "按下时什么都不做",
ActionType::Key => "合成一次键盘按键，可带 cmd/shift/alt/ctrl",
ActionType::Text => "把一段文字输入到当前焦点",
ActionType::OpenApp => "按 bundle id、应用名或路径打开",
ActionType::AppleScript => "跑一段 AppleScript，能驱动几乎所有 mac 应用",
ActionType::Shell => "交给 /bin/sh 执行",
ActionType::VoicePtt => "按住时把麦克风送进虚拟声卡，松手停止",
ActionType::VoiceToggle => "点一下开始送流，再点一下停止",
ActionType::VoiceDictate => "用系统自带的离线语音识别把你说的话转成文字，打进当前焦点。不依赖第三方工具，也不动系统输入设备",
ActionType::VoiceHotkey => {"发一个快捷键去唤起第三方语音输入工具，由它识别并把文字打进当前焦点。短按 = 敲一下，长按 = 按住不放"
}
}
    }
    /// 这个动作需要一个文本参数吗（UI 用来决定是否显示输入框）
    pub fn needs_arg(self) -> bool {
        matches!(
            self,
            ActionType::Text | ActionType::OpenApp | ActionType::AppleScript | ActionType::Shell
        )
    }
}
#[derive(Clone, Serialize, Deserialize, Debug, Default)]
pub struct Action {
    #[serde(rename = "type")]
    pub kind: ActionType,
    /// Key 动作：按键名（如 "up" "return"）
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub key: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub mods: Vec<String>,
    /// Text / OpenApp / AppleScript / Shell 的参数
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub arg: String,
}
impl Action {
    pub fn none() -> Self {
        Self::default()
    }
    pub fn key(k: &str) -> Self {
        Self {
            kind: ActionType::Key,
            key: k.into(),
            ..Default::default()
        }
    }
    pub fn open_app(target: &str) -> Self {
        Self {
            kind: ActionType::OpenApp,
            arg: target.into(),
            ..Default::default()
        }
    }
    pub fn voice_ptt() -> Self {
        Self {
            kind: ActionType::VoicePtt,
            ..Default::default()
        }
    }
    pub fn voice_toggle() -> Self {
        Self {
            kind: ActionType::VoiceToggle,
            ..Default::default()
        }
    }
    /// 外部语音 app 的快捷键。`hold=true` 表示按住期间一直按着。
    pub fn voice_hotkey(key: &str, mods: Vec<String>, hold: bool) -> Self {
        Self {
            kind: ActionType::VoiceHotkey,
            key: key.into(),
            mods,
            arg: if hold { "hold".into() } else { "tap".into() },
        }
    }
    /// UI 上显示的一行摘要
    pub fn describe(&self) -> String {
        match self.kind {
            ActionType::None => "未设置".into(),
            ActionType::Key => {
                let m: String = self.mods.iter().map(|m| format!("{m}+")).collect();
                if self.key.is_empty() {
                    "映射按键（未选）".into()
                } else {
                    format!("{m}{}", self.key)
                }
            }
            ActionType::Text => format!("输入「{}」", ellipsis(&self.arg, 18)),
            ActionType::OpenApp => format!("打开 {}", pretty_app(&self.arg)),
            ActionType::AppleScript => format!("AppleScript · {}", ellipsis(&self.arg, 24)),
            ActionType::Shell => format!("命令 · {}", ellipsis(&self.arg, 24)),
            ActionType::VoicePtt => "按住说话".into(),
            ActionType::VoiceToggle => "开始 / 停止说话".into(),
            ActionType::VoiceDictate => "语音转文字".into(),
            ActionType::VoiceHotkey => {
                let m: String = self.mods.iter().map(|m| format!("{m}+")).collect();
                let k = if self.key.is_empty() {
                    "未选".into()
                } else {
                    format!("{m}{}", self.key)
                };
                let mode = if self.arg == "hold" {
                    "按住"
                } else {
                    "敲一下"
                };
                format!("第三方语音输入 · {k}（{mode}）")
            }
        }
    }
}
fn ellipsis(s: &str, n: usize) -> String {
    let c: Vec<char> = s.chars().collect();
    if c.len() <= n {
        s.to_string()
    } else {
        format!("{}…", c[..n].iter().collect::<String>())
    }
}
/// bundle id 显示成人看得懂的名字
fn pretty_app(t: &str) -> String {
    match t {
        "com.apple.siri.launcher" | "com.apple.Siri" => "Siri".into(),
        "com.anthropic.claudefordesktop" => "Claude".into(),
        "com.openai.codex" | "com.openai.chat" => "ChatGPT".into(),
        "com.google.Chrome" => "Chrome".into(),
        "com.tankxu.firevibe" => "firevibe".into(),
        other => other.rsplit('.').next().unwrap_or(other).to_string(),
    }
}
// ---------------- Profile ----------------
/// 一个位置上的配置：短按 / 长按各一个动作，可整体禁用
#[derive(Clone, Serialize, Deserialize, Debug, Default)]
pub struct SlotAction {
    pub slot: Slot,
    /// 短按：松手时触发
    #[serde(default)]
    pub short: Action,
    /// 长按：按住超过阈值立即触发，并抑制本次短按
    #[serde(default)]
    pub long: Action,
    /// 禁用后这个键完全不响应（也不走自动直通）
    #[serde(default)]
    pub disabled: bool,
}
impl SlotAction {
    pub fn new(slot: Slot, short: Action) -> Self {
        Self {
            slot,
            short,
            long: Action::none(),
            disabled: false,
        }
    }
    pub fn with_long(mut self, long: Action) -> Self {
        self.long = long;
        self
    }
}
#[derive(Clone, Serialize, Deserialize, Debug)]
pub struct Profile {
    pub name: String,
    #[serde(default)]
    pub actions: Vec<SlotAction>,
}
impl Profile {
    pub fn get(&self, s: Slot) -> Option<&SlotAction> {
        self.actions.iter().find(|a| a.slot == s)
    }
    pub fn get_mut(&mut self, s: Slot) -> Option<&mut SlotAction> {
        self.actions.iter_mut().find(|a| a.slot == s)
    }
    /// 取某个位置在指定触发方式下的动作
    pub fn action(&self, s: Slot, long: bool) -> Option<Action> {
        let sa = self.get(s)?;
        if sa.disabled {
            return None;
        }
        let a = if long { &sa.long } else { &sa.short };
        (a.kind != ActionType::None).then(|| a.clone())
    }
    /// 这个位置配了长按吗（决定要不要起长按定时器）
    pub fn has_long(&self, s: Slot) -> bool {
        self.get(s)
            .map(|sa| !sa.disabled && sa.long.kind != ActionType::None)
            .unwrap_or(false)
    }
    pub fn is_disabled(&self, s: Slot) -> bool {
        self.get(s).map(|sa| sa.disabled).unwrap_or(false)
    }
    pub fn set_short(&mut self, s: Slot, a: Action) {
        self.ensure(s).short = a;
    }
    pub fn set_long(&mut self, s: Slot, a: Action) {
        self.ensure(s).long = a;
    }
    pub fn set_disabled(&mut self, s: Slot, v: bool) {
        self.ensure(s).disabled = v;
    }
    fn ensure(&mut self, s: Slot) -> &mut SlotAction {
        if self.get(s).is_none() {
            self.actions.push(SlotAction {
                slot: s,
                ..Default::default()
            });
        }
        self.get_mut(s).unwrap()
    }
    pub fn remove(&mut self, s: Slot) {
        self.actions.retain(|x| x.slot != s);
    }
}
/// 出厂默认这套。TV 键与四个 App 键先摆上，等 usage 学到就能用。
/// 真·默认：一个动作都没有，遥控器完全交还给系统，我们不接管任何键。
/// 想从零开始配的人用这套。
fn blank_profile() -> Profile {
    Profile {
        name: "默认".into(),
        actions: Vec::new(),
    }
}

/// 预配好的那套 —— 开箱能用，所以新装时它是选中的那个。
fn vibe_profile() -> Profile {
    Profile {
        name: "Vibe".into(),
        actions: vec![
            // 麦克风：喂第三方语音输入工具。选纯修饰键是有意的 ——
            // 会走 HID 设备层映射（见 hidremap），那类工具只认硬件来源的按键。
            // 右⌥ 只是个起点，用户得改成自己工具里设的那个热键。
            SlotAction::new(
                Slot::Mic,
                Action::voice_hotkey("rightoption", Vec::new(), false),
            ),
            // 主页 / TV 在 Mac 上没有天然对应动作，摆进列表等配
            SlotAction::new(Slot::Home, Action::none()),
            // 菜单 → 删除键（⌫）
            SlotAction::new(Slot::Menu, Action::key("backspace")),
            SlotAction::new(Slot::Tv, Action::none()),
            SlotAction::new(
                Slot::App1,
                Action::open_app("com.anthropic.claudefordesktop"),
            ),
            SlotAction::new(Slot::App2, Action::open_app("com.openai.codex")),
            SlotAction::new(Slot::App3, Action::open_app("com.google.Chrome")),
            SlotAction::new(Slot::App4, Action::open_app("com.tankxu.firevibe")),
        ],
    }
}
// ---------------- 语音 ----------------
#[derive(Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Debug)]
#[serde(rename_all = "snake_case")]
pub enum VoiceMode {
    Gate,
    Always,
}
#[derive(Clone, Serialize, Deserialize, Debug)]
pub struct VoiceConfig {
    pub enabled: bool,
    pub mode: VoiceMode,
    pub device: String,
    pub gain: f32,
}
impl Default for VoiceConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            mode: VoiceMode::Gate,
            // 自建的那块（driver/build.sh 编的）自称 USB 传输类型 ——
            // 豆包、闪电说会把「虚拟」设备从麦克风候选里滤掉，BlackHole 就是这么被漏掉的。
            // 装了就用它，没装回退 BlackHole（迁移逻辑在 Config::load 里）。
            device: preferred_voice_device(),
            gain: 1.0,
        }
    }
}
/// 优先用我们自己那块（自称 USB，第三方语音工具才认），退回 BlackHole。
/// 只按名字前缀挑，真正的设备查找在 voice/audio 里做。
pub fn preferred_voice_device() -> String {
    const OURS: &str = crate::audiodriver::DEVICE_NAME;
    if crate::audio::input_devices()
        .iter()
        .any(|d| d.name.starts_with(OURS))
    {
        return OURS.into();
    }
    "BlackHole".into()
}

// ---------------- 应用设置 ----------------
#[derive(Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Debug, Default)]
#[serde(rename_all = "snake_case")]
pub enum Lang {
    #[default]
    Zh,
    En,
}
#[derive(Clone, Serialize, Deserialize, Debug)]
pub struct Settings {
    #[serde(default)]
    pub launch_at_login: bool,
    #[serde(default)]
    pub lang: Lang,
    /// 长按判定阈值，毫秒
    #[serde(default = "default_long_ms")]
    pub long_press_ms: u64,
    /// 更新清单地址；空 = 不检查更新
    #[serde(default)]
    pub update_endpoint: String,
    /// 我们处理过的按键，顺手吞掉系统那边的默认行为。
    /// 典型场景：麦克风键的 usage 是 Consumer 0x0221 (AC Search)，
    /// macOS 自己会弹 Spotlight —— 我们打不开独占 HID（要 root），
    /// 只能在事件层拦。需要「辅助功能」权限。
    #[serde(default = "default_true")]
    pub suppress_os_keys: bool,
    /// 学到的「遥控器按键在系统那边对应的事件键码」，之后无条件吞掉。
    /// 自动学习：tap 把最近的非字符事件记下来，HID 线程处理到键时回看 200ms
    /// 把它们认下来 —— 因为系统那条通路可能比我们更快，纯时间窗口拦不住。
    #[serde(default)]
    pub suppress_codes: Vec<i64>,
    /// 【已废弃】原想用 `kCGKeyboardEventKeyboardType` 把屏蔽精确到遥控器，
    /// 但实测那个字段每个事件都不一样，不是稳定的设备 id，学到的全是噪音。
    /// 留着只为兼容老配置，代码里不再读它。
    #[serde(default)]
    pub suppress_kb_types: Vec<i64>,
    /// 说话时自动把系统默认输入切到虚拟声卡，说完切回原来的设备。
    /// 为的是伺候靠系统默认输入做识别的输入法（豆包这类）。
    ///
    /// **默认开**：靠输入法/第三方工具做识别时这是必需的 —— 它们只听系统
    /// 默认输入，没有设备选择器（豆包就是这样）。只在说话期间切走，
    /// 停了 400ms 后还原。实测切换本身 3~13ms。
    ///
    /// 代价：切走期间你的真麦克风对所有 app 失效。进程被硬杀会留在虚拟声卡上，
    /// 下次启动会自动还原（见 `prev_input_id`）。
    #[serde(default = "default_true")]
    pub auto_switch_input: bool,
    /// 语音识别的语言（BCP-47，比如 zh-CN / en-US）
    #[serde(default = "default_stt_locale")]
    pub stt_locale: String,
    /// 识别完自动按一下回车 —— 在 agent 里就是直接把话发出去
    #[serde(default)]
    pub stt_auto_enter: bool,
    /// 切走之前的设备 id。持久化是为了「进程被硬杀」也能在下次启动时还原。
    #[serde(default)]
    pub prev_input_id: Option<u32>,
    /// 最后一次读到的电量。遥控器只是偶尔上报，不存下来的话
    /// 每次重启 app 电量就空着，得等下一次上报才回来。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_battery: Option<i32>,
}
fn default_true() -> bool {
    true
}

fn default_stt_locale() -> String {
    "zh-CN".into()
}
fn default_long_ms() -> u64 {
    350
}
impl Default for Settings {
    fn default() -> Self {
        Self {
            launch_at_login: false,
            lang: Lang::default(),
            long_press_ms: 350,
            update_endpoint: String::new(),
            suppress_os_keys: true,
            suppress_codes: Vec::new(),
            suppress_kb_types: Vec::new(),
            auto_switch_input: true,
            stt_locale: default_stt_locale(),
            stt_auto_enter: false,
            prev_input_id: None,
            last_battery: None,
        }
    }
}
// ---------------- 顶层配置 ----------------
/// 配置结构版本。加一次就会触发一次迁移。
/// 1 = 21 个 HID usage 全部实测完成，旧文件里的猜测值必须刷掉
pub const SCHEMA: u32 = 4;
#[derive(Clone, Serialize, Deserialize, Debug)]
pub struct Config {
    /// 见 [`SCHEMA`]
    #[serde(default)]
    pub schema: u32,
    #[serde(default)]
    pub voice: VoiceConfig,
    #[serde(default)]
    pub exclusive: bool,
    /// 图上位置 -> HID usage（可学习校正）
    #[serde(default = "default_slots")]
    pub slots: Vec<SlotBinding>,
    #[serde(default = "default_profiles")]
    pub profiles: Vec<Profile>,
    #[serde(default)]
    pub active: usize,
    #[serde(default)]
    pub settings: Settings,
}
fn default_profiles() -> Vec<Profile> {
    vec![blank_profile(), vibe_profile()]
}
impl Default for Config {
    fn default() -> Self {
        Self {
            schema: SCHEMA,
            voice: VoiceConfig::default(),
            exclusive: false,
            slots: default_slots(),
            profiles: default_profiles(),
            // 新装默认选 Vibe（预配好的那套），不然开箱什么都不会动
            active: 1,
            settings: Settings::default(),
        }
    }
}
pub fn config_path() -> PathBuf {
    // 自检 / 调试时用 FIREVIBE_CONFIG 指到别处，别动用户真配置。
    // （合成点击测试会真的操作界面并落盘，踩过一次。）
    if let Ok(p) = std::env::var("FIREVIBE_CONFIG") {
        if !p.trim().is_empty() {
            return PathBuf::from(p);
        }
    }
    dirs::config_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("firevibe")
        .join("config.json")
}

impl Config {
    /// 麦克风键要不要在 HID 设备层映射成修饰键，映射成哪个。
    ///
    /// 不做成独立设置项 —— 它就是「第三方语音输入」这个动作的**实现方式**：
    /// 合成的修饰键带着「进程合成标记」，只认硬件来源的工具收不到，
    /// 所以配了纯修饰键当热键时，直接在设备层把这颗键变成它。
    /// 同一件事只在一个地方配。
    pub fn mic_remap_key(&self) -> Option<String> {
        let p = self.profile();
        for a in &p.actions {
            if a.disabled || a.slot != crate::layout::Slot::Mic {
                continue;
            }
            for act in [&a.short, &a.long] {
                if act.kind == ActionType::VoiceHotkey
                    && act.mods.is_empty()
                    && crate::hidremap::usage_of(&act.key).is_some()
                {
                    return Some(act.key.clone());
                }
            }
        }
        None
    }

    pub fn load() -> Self {
        let mut c: Config = std::fs::read_to_string(config_path())
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default();

        if c.voice.gain <= 0.0 {
            c.voice.gain = 1.0;
        }

        if c.profiles.is_empty() {
            c.profiles = default_profiles();
        }

        if c.active >= c.profiles.len() {
            c.active = 0;
        }

        // 迁移：schema 0 的文件里 slots 存的是当年推测的 usage

        // （TV 记成 0x0089、返回记成 Esc、四个 App 键根本没有），

        // 现在 21 个全部实测过了，整表刷新。

        if c.schema < SCHEMA {
            if c.schema < 1 {
                c.slots = default_slots();
            }
            // schema 2：老配置停在旧默认值 "BlackHole" 上。装了我们自己那块
            // （自称 USB、第三方语音工具才认得）就换过去；只动这个确切的旧默认值，
            // 用户自己改过的名字不碰。
            if c.schema < 2 && c.voice.device == "BlackHole" {
                c.voice.device = preferred_voice_device();
            }
            // schema 3：设备名从「Fire Vibe Mic」改成「FireVibe Mic」
            if c.schema < 3 && c.voice.device == "Fire Vibe Mic" {
                c.voice.device = crate::audiodriver::DEVICE_NAME.into();
            }
            // schema 4：原来那套叫「默认」，其实是预配过的 —— 改名 Vibe，
            // 再补一套真正什么都没配的「默认」。用户当时选的还是同一套，
            // 所以 active 跟着往后挪一位。
            if c.schema < 4 && !c.profiles.iter().any(|p| p.name == "Vibe") {
                if let Some(first) = c.profiles.first_mut() {
                    if first.name == "默认" || first.name == "Default" {
                        first.name = "Vibe".into();
                        c.profiles.insert(0, blank_profile());
                        c.active += 1;
                    }
                }
            }
            c.schema = SCHEMA;
            let _ = c.save();
        }
        c
    }

    pub fn save(&self) -> Result<()> {
        let p = config_path();

        if let Some(d) = p.parent() {
            std::fs::create_dir_all(d)?;
        }
        std::fs::write(p, serde_json::to_string_pretty(self)?)?;
        Ok(())
    }

    // ---- profile ----

    pub fn profile(&self) -> &Profile {
        &self.profiles[self.active.min(self.profiles.len() - 1)]
    }

    pub fn profile_mut(&mut self) -> &mut Profile {
        let i = self.active.min(self.profiles.len() - 1);
        &mut self.profiles[i]
    }

    pub fn add_profile(&mut self, name: impl Into<String>) {
        self.profiles.push(Profile {
            name: name.into(),
            actions: Vec::new(),
        });
        self.active = self.profiles.len() - 1;
    }

    pub fn remove_profile(&mut self, i: usize) {
        if self.profiles.len() <= 1 || i >= self.profiles.len() {
            return;
        }
        self.profiles.remove(i);

        if self.active >= self.profiles.len() {
            self.active = self.profiles.len() - 1;
        }
    }

    // ---- slot <-> usage ----

    pub fn slot_key(&self, s: Slot) -> Option<Key> {
        self.slots.iter().find(|x| x.slot == s).map(|x| x.key)
    }

    /// HID usage 反查它在图上的位置

    pub fn key_slot(&self, k: Key) -> Option<Slot> {
        self.slots.iter().find(|x| x.key == k).map(|x| x.slot)
    }

    pub fn set_slot(&mut self, s: Slot, k: Key) {
        self.slots.retain(|x| x.slot != s && x.key != k);
        self.slots.push(SlotBinding { slot: s, key: k });
    }

    /// 某个 usage 在指定触发方式下该执行什么

    pub fn action_for(&self, k: Key, long: bool) -> Option<(Slot, Action)> {
        let s = self.key_slot(k)?;
        self.profile().action(s, long).map(|a| (s, a))
    }

    /// 长按阈值（毫秒）

    pub fn long_press_ms(&self) -> u64 {
        self.settings.long_press_ms.max(120)
    }
}
