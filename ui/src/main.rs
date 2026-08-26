//! firevibe —— Fire TV 遥控器控制台。
//! 单页：左侧软遥控器，右侧状态条 + 方案 + 自定义操作；右上角进设置。
//! 版式与配色一律以 `design/mockup.html` 为准。
mod assets;
mod cards;
mod editor;
mod hud;
mod i18n;
mod remote;
mod settings;
mod stats;
mod theme;
mod widget;

use firevibe_core::{
    config::{ActionType, Config, Lang},
    keys::Key,
    layout::Slot,
    runtime::{Event, Runtime},
    update::UpdateStatus,
    audio::{self, InputDevice},
    voice::{loopback_status, LoopbackStatus},
};
use gpui::{
    deferred, div, prelude::*, px, relative, size, App, Application, Bounds, Context, Entity,
    Menu, MenuItem, SharedString, Window, WindowBounds, WindowOptions,
};

// 退出动作 —— 菜单栏和 Cmd-Q 都触发它。macOS 上 close 被改成隐藏（见 open_window
// 里的 on_window_should_close），所以必须有个明确的退出入口，否则 app 退不掉。
gpui::actions!(firevibe, [Quit]);
use gpui_component::{input::InputState, Root};
use remote::COL_LEFT_W;

/// 顶部拖拽条高度。红绿灯浮在这条里，内容从它下面开始 ——
/// 不能把状态卡也塞进这条：窗口窄的时候居中容器的左边缘会撞上红绿灯。
const TOPBAR_H: f32 = 40.;
/// 内容整体最大宽度，超过就居中留白
const CONTENT_MAX_W: f32 = 1280.;
/// 卡片 hover 过渡时长
const HOVER_MS: Duration = Duration::from_millis(140);
use std::collections::HashSet;
use std::sync::atomic::Ordering;
use std::sync::mpsc::Receiver;
use std::sync::Arc;
use std::time::{Duration, Instant};
use theme::*;
use widget::*;

#[derive(PartialEq, Eq, Clone, Copy)]
pub enum Screen {
    Main,
    Settings,
    Stats,
}

/// 编辑弹窗的临时状态。保存时才写回配置。
pub struct EditState {
    pub slot: Slot,
    pub long: bool,
    pub kind: ActionType,
    pub key: String,
    pub mods: Vec<String>,
    /// 「外部语音 app」的触发模式：true = 按住期间保持按下
    pub hold: bool,
    /// 「外部语音 app」短按时用双击而不是单击。豆包默认就是「双击开、单击关」，
    /// 单击它压根不进收音状态 —— 这是目标 app 的约定，只能由用户按实际情况选。
    pub dbl: bool,
    /// 文本参数（打开应用 / AppleScript / 命令 / 输入文字；HTTP 时是 URL）
    pub input: Entity<InputState>,
    /// HTTP：方法是不是 POST（否则 GET）
    pub post: bool,
    /// HTTP：POST 请求体
    pub body_in: Entity<InputState>,
    /// HTTP：重试次数
    pub retries_in: Entity<InputState>,
    /// HTTP：超时毫秒
    pub timeout_in: Entity<InputState>,
    /// 热键录制用的焦点句柄。要收键盘事件，元素必须 track_focus 且被聚焦。
    pub focus: gpui::FocusHandle,
    /// 正在等你按组合键
    pub recording: bool,
    /// 录制期间的 tap 会话。走 tap 而不是窗口事件 —— 要录的组合键
    /// 可能已经被别的软件注册成全局热键，那样窗口这边永远收不到。
    /// drop 即停止（键盘恢复正常）。
    pub grab: Option<firevibe_core::hotkey::Capture>,
}

pub struct FireVibe {
    pub rt: Arc<Runtime>,
    rx: Receiver<Event>,
    pub screen: Screen,
    /// 编辑弹窗
    pub dialog: Option<EditState>,
    /// 展开了「…」菜单的卡片
    pub menu_open: Option<Slot>,
    /// 鼠标停在哪张卡上（决定测试/编辑按钮是否露出）
    pub hover_card: Option<Slot>,
    /// 正在淡出的那张卡（hover 移走后还要放完动画）
    pub hover_from: Option<Slot>,
    /// 上次 hover 变化的时刻，过渡进度由它算
    pub hover_at: Instant,
    /// 鼠标正按住图上哪个按键
    pub mouse_down: Option<Slot>,
    pub profile_open: bool,
    /// 「添加按键」的位置选择面板
    pub adding: bool,
    /// 实体按下的键，图上跟着亮
    pub pressed: HashSet<Key>,
    /// 软点击的短暂按下效果
    pub soft: Option<(Slot, Instant)>,
    /// 语音链路是否已就绪 / 正在建
    voice_ready: bool,
    voice_rx: Option<Receiver<Result<(), String>>>,
    /// 系统默认输入设备 + 可选列表（后台线程刷新，CoreAudio 会跑 run loop）
    pub audio_cur: Option<InputDevice>,
    pub audio_list: Vec<InputDevice>,
    audio_rx: Option<Receiver<(Option<InputDevice>, Vec<InputDevice>)>>,
    audio_at: Instant,
    /// 系统输入下拉是否展开
    pub input_open: bool,
    /// 设置页里的语音识别语言下拉是否展开；语言列表来自 Speech.framework，
    /// 与 FireVibe 自己的中/英文界面语言完全独立。
    pub stt_locale_open: bool,
    pub stt_locales: Vec<firevibe_core::stt::SpeechLocale>,
    /// 装虚拟声卡前的说明弹窗。系统那个授权框只写「osascript wants to
    /// make changes」，署名还是个陌生进程 —— 直接弹给人输密码是不合格的，
    /// 先把要做什么讲清楚。
    /// 电量读取已经起过了吗（后台线程，定时跑 bundle 里的 battprobe）
    batt_started: bool,
    /// 方案改名弹窗：装着输入框，None = 没在改
    pub renaming: Option<Entity<InputState>>,
    pub install_confirm: bool,
    /// 「配对新遥控器」弹窗开着
    pub pairing: bool,
    /// 扫到的 HID 设备列表（后台线程填）；None=还在扫
    pair_devices: Option<Vec<firevibe_core::device::HidDev>>,
    pair_rx: Option<std::sync::mpsc::Receiver<Vec<firevibe_core::device::HidDev>>>,
    /// 「测试输入」面板是否打开
    pub voice_test: bool,
    /// 面板里正在按住测试（送流到虚拟声卡）
    pub testing: bool,
    /// 面板里正在按住听写
    pub dictating: bool,
    /// 最近一次听写结果，面板里显示
    pub last_stt: Option<String>,
    /// 按住测试开始时的帧数，用来算这次收了多少帧
    test_frames0: u64,
    /// 菜单刚被「点外面」关掉的时刻。触发器在这之后很短时间内忽略点击，
    /// 否则捕获阶段先关、冒泡阶段触发器又把它打开，等于关不掉。
    dismiss_at: Instant,
    pub loopback: LoopbackStatus,
    loopback_rx: Option<Receiver<LoopbackStatus>>,
    loopback_at: Instant,
    /// HID 是否已经尝试启动过（挪出构造期，见 new 的注释）
    started: bool,
    /// 上次尝试开 HID 的时刻，用来控制自动重连节奏
    hid_try_at: Instant,
    /// 听写时屏幕底部那条悬浮电平窗
    hud: Option<gpui::WindowHandle<hud::Hud>>,
    pub update: UpdateStatus,
    update_rx: Option<Receiver<UpdateStatus>>,
    /// 原生文件面板异步返回的配置导入/导出路径。同步 runModal 会启动嵌套
    /// AppKit 事件循环并让 GPUI 退出，所以结果统一在 pump 里消费。
    pub config_import_rx: Option<Receiver<Option<std::path::PathBuf>>>,
    pub config_export_rx: Option<Receiver<Option<std::path::PathBuf>>>,
    /// 一次性提示
    pub toast: Option<(String, Instant)>,
    pub product: String,
    pub err: Option<String>,
    /// 自检用：`FIREVIBE_BOOT=settings` 或 `FIREVIBE_BOOT=dialog:app1:long`
    /// 直接把界面拉到某一屏，方便截图核对设计稿。首帧消费掉。
    boot: Option<String>,
    /// 启动后自动查一次更新（只查一次）
    boot_update_checked: bool,
    /// 用户主动触发的错误（点 Connect 失败）——后台自动重连不清它，用户手动关才清
    err_sticky: bool,
    /// 首次引导弹窗（权限/声卡）
    onboarding: bool,
}

impl FireVibe {
    fn new(cx: &mut Context<Self>) -> Self {
        let cfg = Config::load();
        let stt_locales = firevibe_core::stt::supported_locales();
        let (rt, rx) = Runtime::new(cfg);
        let rt = Arc::new(rt);
        // 自测钩子：FIREVIBE_REC_TEST=<延迟秒>,<时长秒> —— 到点自己录一段再停
        if let Ok(v) = std::env::var("FIREVIBE_REC_TEST") {
            let mut it = v.split(',').filter_map(|x| x.trim().parse::<f32>().ok());
            let delay = it.next().unwrap_or(6.0);
            let dur = it.next().unwrap_or(4.0);
            let rt2 = rt.clone();
            std::thread::spawn(move || {
                std::thread::sleep(Duration::from_secs_f32(delay));
                match firevibe_core::recorder::Rec::start(firevibe_core::voice::OPUS_RATE) {
                    Ok(r) => {
                        *rt2.recording.lock() = Some(r);
                        rt2.status.mic_on.store(true, Ordering::Relaxed);
                        eprintln!("[rectest] 开始录音");
                    }
                    Err(e) => {
                        eprintln!("[rectest] 起不来: {e}");
                        return;
                    }
                }
                // FIREVIBE_REC_WAV：把一个 16k/单声道 WAV 当成遥控器音频灌进去，
                // 这样「录音 → 写盘 → 文件名/时长」整条链路不用真按遥控器就能验
                if let Ok(wav) = std::env::var("FIREVIBE_REC_WAV") {
                    match std::fs::read(&wav) {
                        Ok(b) if b.len() > 44 => {
                            let pcm: Vec<i16> = b[44..]
                                .chunks_exact(2)
                                .map(|c| i16::from_le_bytes([c[0], c[1]]))
                                .collect();
                            if let Some(r) = rt2.recording.lock().as_mut() {
                                r.push(&pcm);
                            }
                            eprintln!(
                                "[rectest] 灌入 {} 采样（{:.2}s）",
                                pcm.len(),
                                pcm.len() as f32 / 16000.0
                            );
                        }
                        Ok(_) => eprintln!("[rectest] WAV 太小"),
                        Err(e) => eprintln!("[rectest] 读 WAV 失败 {e}"),
                    }
                }
                for i in 0..(dur as u32).max(1) {
                    std::thread::sleep(Duration::from_secs(1));
                    eprintln!("[rectest] 第 {}s，准备取锁", i + 1);
                    eprintln!(
                        "[rectest] {}s 音频帧={} 已写={:.2}s",
                        i + 1,
                        rt2.status.audio_frames.load(Ordering::Relaxed),
                        rt2.recording.lock().as_ref().map(|r| r.seconds()).unwrap_or(-1.0)
                    );
                }
                rt2.status.mic_on.store(false, Ordering::Relaxed);
                // 注意别在持锁时调 finish —— 先 take 出来再单独收尾
                let taken = rt2.recording.lock().take();
                if let Some(r) = taken {
                    match r.finish() {
                        Ok((p, s)) => eprintln!("[rectest] 存好 {} （{s:.2}s）", p.display()),
                        Err(e) => eprintln!("[rectest] 收尾失败 {e}"),
                    }
                }
            });
        }
        // 自测钩子：FIREVIBE_TYPE_TEST=<秒>,<文字> —— 到点从本进程打一段字，
        // 用来把「注入」和「识别」分开定位
        if let Ok(v) = std::env::var("FIREVIBE_TYPE_TEST") {
            let (d, t) = v.split_once(',').unwrap_or(("6", "测试一二三"));
            let delay: f32 = d.trim().parse().unwrap_or(6.0);
            let text = t.to_string();
            let rt2 = rt.clone();
            std::thread::spawn(move || {
                std::thread::sleep(Duration::from_secs_f32(delay));
                eprintln!("[typetest] 辅助功能={} 打 {:?}", rt2.inj.available(), text);
                eprintln!("[typetest] 结果 {:?}", rt2.inj.type_text(&text));
            });
        }
        // 自测钩子：FIREVIBE_DICT_TEST=<延迟秒>,<时长秒>
        // 到点自己开一次听写再关掉，这样不用点界面（点了 FireVibe 就成前台，
        // 前台一变就测不出「打字到底进了哪个 app」）。
        if let Some(v) = std::env::var("FIREVIBE_DICT_TEST").ok() {
            let mut it = v.split(',').filter_map(|x| x.trim().parse::<f32>().ok());
            let delay = it.next().unwrap_or(8.0);
            let dur = it.next().unwrap_or(3.0);
            let rt2 = rt.clone();
            std::thread::spawn(move || {
                std::thread::sleep(Duration::from_secs_f32(delay));
                eprintln!("[dicttest] 开始听写 → {}", rt2.set_dictating(true));
                // FIREVIBE_DICT_WAV：把一个 16k/单声道 WAV 当成遥控器的音频灌进去，
                // 这样识别→打字→落到哪个 app 整条链路都能自测，不用真按遥控器
                if let Ok(wav) = std::env::var("FIREVIBE_DICT_WAV") {
                    match std::fs::read(&wav) {
                        Ok(b) if b.len() > 44 => {
                            let pcm: Vec<i16> = b[44..]
                                .chunks_exact(2)
                                .map(|c| i16::from_le_bytes([c[0], c[1]]))
                                .collect();
                            if let Some(r) = rt2.dictating.lock().as_mut() {
                                r.push(&pcm);
                            }
                            eprintln!("[dicttest] 灌入 {} 采样（{:.1}s）", pcm.len(), pcm.len() as f32 / 16000.0);
                        }
                        Ok(_) => eprintln!("[dicttest] WAV 太小"),
                        Err(e) => eprintln!("[dicttest] 读 WAV 失败 {e}"),
                    }
                }
                std::thread::sleep(Duration::from_secs_f32(dur));
                eprintln!("[dicttest] 结束听写 → {}", rt2.set_dictating(false));
            });
        }
        // 必须先 await 再 update —— 构造期间 app 的 RefCell 正被借用
        // 必须先 await 再 update —— 构造期间 app 的 RefCell 正被借用。
        // 平时 70ms 够了（只是轮询状态）；有过渡在跑时切到 16ms，
        // 不然动画只有 14fps，一格一格的。
        cx.spawn(async move |this, cx| {
            let mut fast = false;
            loop {
                let ms = if fast { 16 } else { 70 };
                cx.background_executor().timer(Duration::from_millis(ms)).await;
                match this.update(cx, |v, cx| {
                    v.pump();
                    // 开/关悬浮窗必须在绘制过程之外做，
                    // 在 render() 里调 open_window 会重入 GPUI 的绘制、直接把进程带走
                    v.sync_hud(cx);
                    cx.notify();
                    v.animating()
                }) {
                    Ok(f) => fast = f,
                    Err(_) => break,
                }
            }
        })
        .detach();
        let show_onb = !rt.cfg.read().settings.onboarded;
        Self {
            rt,
            rx,
            screen: Screen::Main,
            dialog: None,
            menu_open: None,
            hover_card: None,
            hover_from: None,
            hover_at: Instant::now(),
            mouse_down: None,
            profile_open: false,
            adding: false,
            pressed: HashSet::new(),
            soft: None,
            voice_ready: false,
            voice_rx: None,
            audio_cur: None,
            audio_list: Vec::new(),
            audio_rx: None,
            audio_at: Instant::now() - Duration::from_secs(10),
            input_open: false,
            stt_locale_open: false,
            stt_locales,
            batt_started: false,
            renaming: None,
            install_confirm: false,
            pairing: false,
            pair_devices: None,
            pair_rx: None,
            voice_test: false,
            testing: false,
            dictating: false,
            last_stt: None,
            test_frames0: 0,
            dismiss_at: Instant::now() - Duration::from_secs(10),
            loopback: LoopbackStatus::Unknown,
            loopback_rx: None,
            // 减 10 秒：让首帧之后立刻查一次，而不是等 3 秒
            loopback_at: Instant::now() - Duration::from_secs(10),
            started: false,
            hid_try_at: Instant::now() - Duration::from_secs(10),
            hud: None,
            update: UpdateStatus::Idle,
            update_rx: None,
            config_import_rx: None,
            config_export_rx: None,
            toast: None,
            product: String::new(),
            err: None,
            boot: std::env::var("FIREVIBE_BOOT").ok(),
            boot_update_checked: false,
            err_sticky: false,
            onboarding: show_onb,
        }
    }

    /// 首帧处理 FIREVIBE_BOOT
    fn consume_boot(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(b) = self.boot.take() else { return };
        let mut it = b.split(':');
        match it.next() {
            Some("settings") => self.screen = Screen::Settings,
            Some("stats") => self.screen = Screen::Stats,
            Some("onboarding") => self.onboarding = true,
            Some("dialog") => {
                let slot = it.next().unwrap_or("app1");
                let long = it.next() == Some("long");
                let s = Slot::ALL.into_iter().find(|x| x.id() == slot).unwrap_or(Slot::App1);
                self.open_editor(s, long, window, cx);
            }
            Some("add") => self.adding = true,
            // 悬停 / 菜单 / 方案下拉这几个状态只在交互中出现，
            // 自检时没法合成鼠标事件，就直接把状态摆出来截图核对
            Some("hover") => {
                let id = it.next().unwrap_or("app1");
                self.hover_card = Slot::ALL.into_iter().find(|x| x.id() == id);
            }
            Some("menu") => {
                let id = it.next().unwrap_or("app2");
                let s = Slot::ALL.into_iter().find(|x| x.id() == id);
                self.hover_card = s;
                self.menu_open = s;
            }
            // 自检：用 say 合成一段语音直接送去识别。裸测试进程没有 Info.plist
            // 拿不到语音识别权限，只能在打好包的 app 里验。
            Some("sttest") => {
                let rt = self.rt.clone();
                std::thread::spawn(move || {
                    if !firevibe_core::stt::authorized() {
                        let _ = firevibe_core::stt::request_auth();
                    }
                    let wav = std::env::temp_dir().join("firevibe-say-test.wav");
                    let ok = std::process::Command::new("say")
                        .args(["-v", "Tingting", "--data-format=LEI16@16000", "-o"])
                        .arg(&wav)
                        .arg("今天天气很好")
                        .status()
                        .map(|s| s.success())
                        .unwrap_or(false);
                    let msg = if !ok {
                        "say 合成失败".to_string()
                    } else {
                        match firevibe_core::stt::transcribe_file(&wav, "zh-CN", true) {
                            Ok(t) => format!("自检：说「今天天气很好」→ 识别「{t}」"),
                            Err(e) => format!("自检失败: {e}"),
                        }
                    };
                    eprintln!("[sttest] {msg}");
                    let _ = rt.log(msg);
                });
            }
            Some("profile") => self.profile_open = true,
            Some("input") => self.input_open = true,
            Some("vtest") => {
                self.voice_test = true;
                self.test_frames0 = self.rt.status.audio_frames.load(Ordering::Relaxed);
            }
            _ => {}
        }
    }

    pub fn lang(&self) -> Lang {
        self.rt.cfg.read().settings.lang
    }
    pub fn l(&self) -> i18n::L {
        i18n::L(self.lang())
    }

    pub fn just_dismissed_pub(&self) -> bool {
        self.just_dismissed()
    }
    pub fn dismiss_menus_pub(&mut self) {
        self.dismiss_menus()
    }

    /// 听写时开屏幕底部那条悬浮电平窗，停了就关。
    /// 放在 render 里而不是 pump —— 开窗需要 `&mut App`，pump 里拿不到。
    fn sync_hud(&mut self, cx: &mut Context<Self>) {
        // 听写在收音，或者麦克风开着喂第三方输入法 —— 两种都该有电平反馈
        let on = self.rt.cfg.read().settings.show_level_hud
            && (self.rt.dictating.lock().is_some()
                || self.rt.status.mic_on.load(Ordering::Relaxed));
        match (on, self.hud.is_some()) {
            (true, false) => self.hud = hud::open(self.rt.clone(), cx),
            (false, true) => {
                if let Some(h) = self.hud.take() {
                    hud::close(&h, cx);
                }
            }
            _ => {}
        }
    }

    /// 菜单类触发器要不要忽略这次点击（刚被点外关掉的那一瞬）
    fn just_dismissed(&self) -> bool {
        self.dismiss_at.elapsed() < Duration::from_millis(180)
    }

    /// 关掉所有下拉/菜单，并记下时刻
    fn dismiss_menus(&mut self) {
        self.profile_open = false;
        self.menu_open = None;
        self.input_open = false;
        self.stt_locale_open = false;
        self.dismiss_at = Instant::now();
    }

    pub fn toast(&mut self, s: impl Into<String>) {
        self.toast = Some((s.into(), Instant::now()));
    }

    /// 还有动画在跑吗 —— 决定下一帧的间隔
    pub fn animating(&self) -> bool {
        self.rt.dictating.lock().is_some()
            || self.voice_test
            || self.soft.is_some()
            || ((self.hover_card.is_some() || self.hover_from.is_some())
                && self.hover_at.elapsed() < HOVER_MS + Duration::from_millis(30))
    }

    /// 卡片 hover 过渡进度：1 = 完全 hover 态
    pub fn card_t(&self, slot: Slot) -> f32 {
        let p = ease_out(self.hover_at.elapsed().as_secs_f32() / HOVER_MS.as_secs_f32());
        if self.hover_card == Some(slot) {
            p
        } else if self.hover_from == Some(slot) {
            1. - p
        } else {
            0.
        }
    }

    /// hover 目标变了：记下上一张，重置计时
    pub fn set_hover(&mut self, slot: Option<Slot>) {
        if self.hover_card == slot {
            return;
        }
        self.hover_from = self.hover_card;
        self.hover_card = slot;
        self.hover_at = Instant::now();
    }

    fn pump(&mut self) {
        // 临时自测：FIREVIBE_PAIRTEST=1 时启动 2 秒后自动开配对（模拟真实点击时机）
        if std::env::var_os("FIREVIBE_PAIRTEST").is_some()
            && !self.pairing && self.pair_devices.is_none()
            && self.started
        {
            // 复用 boot_update_checked 之外的一次性——用 pairing 本身没触发过判断
            static ONCE: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
            if !ONCE.swap(true, std::sync::atomic::Ordering::Relaxed) {
                // 这里没有 cx，只能置标志 + 起线程，模拟 open_pairing 的效果
                self.pairing = true;
                let (tx, rx) = std::sync::mpsc::channel();
                self.pair_rx = Some(rx);
                std::thread::spawn(move || { let _ = tx.send(firevibe_core::device::list_hid()); });
            }
        }
        self.poll_hotkey_grab();
        self.poll_battery();
        self.poll_config_file_io();
        // 启动后自动查一次更新（GitHub Releases），不用等用户去设置里点
        if !self.boot_update_checked {
            self.boot_update_checked = true;
            self.check_update();
        }
        // 淡出放完就把它摘掉，免得一直被当成「在动」
        if self.hover_from.is_some() && self.hover_at.elapsed() > HOVER_MS {
            self.hover_from = None;
        }
        if let Some((_, t)) = self.soft {
            if t.elapsed() > Duration::from_millis(160) {
                self.soft = None;
            }
        }
        if let Some((_, t)) = self.toast {
            if t.elapsed() > Duration::from_millis(2200) {
                self.toast = None;
            }
        }
        // HID 打开也会跑 run loop，同样不能放构造期。
        // 「设备没连上」是正常状态不是错误 —— 不弹错误条，靠状态卡高亮表示，
        // 后台每 2 秒自己重试，遥控器一醒就自动连上。
        if !self.started || (!self.connected() && self.hid_try_at.elapsed() > Duration::from_secs(2))
        {
            let first = !self.started;
            self.started = true;
            self.hid_try_at = Instant::now();
            match self.rt.start() {
                Ok(_) => {
                    self.err = None;
                    self.err_sticky = false; // 连上了，清掉主动错误
                }
                Err(e) => {
                    // 用户主动点过 Connect 报的错要留着；后台自动重连不覆盖、不清
                    if !self.err_sticky {
                        let m = format!("{e:#}");
                        // 只有真错误才报；没连上就安静等
                        self.err = if m.starts_with("HID_NOT_FOUND") { None } else { Some(m) };
                    }
                }
            }
            if first {
                // 启动时把关键权限状态打到 stderr，排障时一眼能看到
                eprintln!(
                    "[firevibe] 按键注入(辅助功能)={} 语音识别={}",
                    self.rt.inj.available(),
                    firevibe_core::stt::auth_status()
                );
                // 上次异常退出可能把系统输入留在虚拟声卡上，开机先补救一下
                self.rt.recover_input();
                // 电量跟踪器要在主线程建
                if !self.batt_started {
                    self.batt_started = true;
                    // 每 5 分钟读一次 —— 电量变化慢，没必要频繁连蓝牙
                    firevibe_core::battery::spawn_tracker(300);
                }
                // 按配置重下 HID 层映射：设了就下（幂等，顺带盖掉上次残留），没设就清
                if let Some(m) = self.rt.sync_hid_remap() {
                    eprintln!("[firevibe] {m}");
                }
                // 事件 tap：吞掉遥控器按键在系统那边的默认行为（麦克风键弹 Spotlight）
                if let Err(e) = self.rt.start_tap() {
                    let m = self.l().toast_block_failed(&e.to_string());
                    self.toast(m);
                }
            }
        }
        // 语音链路：虚拟声卡就绪后把 sink 建起来。**必须后台线程** ——
        // cpal 打开设备会跑 run loop，在 update 里同步做会触发那个 RefCell panic。
        // 注意只建链路不开麦，开麦是按需的（热麦克风会让蓝灯一直闪还费电）。
        if let Some(rx) = &self.voice_rx {
            if let Ok(r) = rx.try_recv() {
                self.voice_rx = None;
                match r {
                    Ok(()) => self.voice_ready = true,
                    Err(e) => { let m = self.l().toast_voice_start_failed(&e.to_string()); self.toast(m); }
                }
            }
        } else if !self.voice_ready && self.loopback.is_ready() {
            let rt = self.rt.clone();
            let (tx, rx) = std::sync::mpsc::channel();
            self.voice_rx = Some(rx);
            std::thread::spawn(move || {
                let _ = tx.send(rt.ensure_voice().map_err(|e| format!("{e:#}")));
            });
        }
        // 配对弹窗的设备扫描结果
        if let Some(rx) = &self.pair_rx {
            if let Ok(devs) = rx.try_recv() {
                self.pair_devices = Some(devs);
                self.pair_rx = None;
            }
        }
        // 虚拟声卡状态：cpal 枚举 CoreAudio 是同步阻塞且会跑 run loop 的，
        // 丢后台线程，结果走 channel 回来
        if let Some(rx) = &self.loopback_rx {
            if let Ok(st) = rx.try_recv() {
                self.loopback = st;
                self.loopback_rx = None;
                self.loopback_at = Instant::now();
            }
        } else if self.loopback_at.elapsed() > Duration::from_secs(3) {
            let dev = self.rt.cfg.read().voice.device.clone();
            let (tx, rx) = std::sync::mpsc::channel();
            self.loopback_rx = Some(rx);
            std::thread::spawn(move || {
                let _ = tx.send(loopback_status(&dev));
            });
        }
        // 实体按下状态
        self.pressed = self.rt.pressed.lock().clone();
        // 更新检查结果
        if let Some(rx) = &self.update_rx {
            if let Ok(st) = rx.try_recv() {
                self.update = st;
                self.update_rx = None;
            }
        }
        // 音频设备同样丢后台线程 —— CoreAudio 调用会跑 run loop
        if let Some(rx) = &self.audio_rx {
            if let Ok((cur, list)) = rx.try_recv() {
                self.audio_cur = cur;
                self.audio_list = list;
                self.audio_rx = None;
                self.audio_at = Instant::now();
            }
        } else if self.audio_at.elapsed() > Duration::from_secs(2) {
            let (tx, rx) = std::sync::mpsc::channel();
            self.audio_rx = Some(rx);
            std::thread::spawn(move || {
                let _ = tx.send((audio::default_input(), audio::input_devices()));
            });
        }
        while let Ok(ev) = self.rx.try_recv() {
            match ev {
                // 之前这个分支压根没有，按遥控器时界面一个字都不显示 ——
                // 包括「没有语音识别权限」这种关键报错，全被丢掉了
                Event::Key { down, result, .. } => {
                    if down && !result.is_empty() {
                        self.toast(result);
                    }
                }
                Event::Connected { product, .. } => {
                    // 电量按连上的这台设备的名字读 —— 换了遥控器要跟着换目标，
                    // 否则一直读不到、界面显示上一台的陈旧电量
                    firevibe_core::battery::set_target(&product);
                    self.product = product;
                }
                Event::Disconnected(e) => self.err = Some(e),
                Event::Log(s) => {
                    if let Some(t) = s.strip_prefix("听写（").and_then(|r| r.split_once("）：")) {
                        self.last_stt = Some(t.1.to_string());
                        self.toast(format!("听写：{}", t.1));
                    } else if s.starts_with("没识别出内容") || s.starts_with("听写失败") {
                        self.last_stt = Some(s.clone());
                        self.toast(s.clone());
                    } else if s.starts_with("已学到") {
                        self.toast(s);
                    } else if s.contains("失败") || s.contains("error") {
                        self.err = Some(s);
                    }
                }
                _ => {}
            }
        }
    }

    /// 点了图上的按键：跑它配的操作，并做一次按下动效
    pub fn tap(&mut self, slot: Slot, cx: &mut Context<Self>) {
        self.soft = Some((slot, Instant::now()));
        let r = self.rt.trigger_slot(slot, false);
        if !r.is_empty() {
            self.toast(r);
        }
        cx.notify();
    }

    pub fn connected(&self) -> bool {
        self.rt.status.connected.load(Ordering::Relaxed)
    }

    pub fn battery(&self) -> i32 {
        self.rt.status.battery.load(Ordering::Relaxed)
    }

    pub fn save(&self) {
        let _ = self.rt.cfg.read().save();
        // 硬件层映射是从动作配置推导出来的，动作一改就得跟着重下 / 清掉
        let _ = self.rt.sync_hid_remap();
    }

    pub fn check_update(&mut self) {
        if matches!(self.update, UpdateStatus::Checking) {
            return;
        }
        let ep = self.rt.cfg.read().settings.update_endpoint.clone();
        let (tx, rx) = std::sync::mpsc::channel();
        self.update = UpdateStatus::Checking;
        self.update_rx = Some(rx);
        std::thread::spawn(move || {
            let st = firevibe_core::update::check(if ep.is_empty() { None } else { Some(&ep) });
            let _ = tx.send(st);
        });
    }

    // ── 顶栏：品牌在左，设置在右 ──
    fn header(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let l = self.l();
        div()
            .flex()
            .items_center()
            .gap(px(12.))
            // 列间距是 22，设计稿里 .top 的 margin-bottom 是 24，补这 2px
            .mb(px(2.))
            .child(
                // 拖拽区：标题 + 右侧空白（flex_1 撑满到齿轮前）。按下即用
                // performWindowDragWithEvent 原生拖窗；齿轮是单独兄弟节点，不在这里，
                // 所以点齿轮不会误触发拖拽，输入框那些也完全不受影响。
                div()
                    .id("hdr-drag")
                    .flex_1()
                    .flex()
                    .flex_col()
                    .on_mouse_down(gpui::MouseButton::Left, |_, _, _| {
                        firevibe_core::tray::start_window_drag();
                    })
                    .child(
                        div()
                            .text_size(px(19.))
                            .font_weight(w(640.))
                            .text_color(c(INK))
                            .child("FireVibe"),
                    )
                    .child(
                        div()
                            .text_size(px(12.))
                            .text_color(c(INK3))
                            .child(SharedString::from(l.app_sub())),
                    ),
            )
            .child(icon_btn("gear", "settings").on_click(cx.listener(|this, _, _, cx| {
                this.screen = Screen::Settings;
                cx.notify();
            })))
    }

    /// 右栏最上面那块：状态卡 +（出错时）警示条
    fn status_row(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let cards = self.status_cards(cx);
        // 配了「语音转文字」但没授权 —— 这个必须在界面上说出来。
        // 之前只在按下遥控器时返回一句被丢掉的错误字符串，等于完全不可见。
        let needs_stt = self
            .rt
            .cfg
            .read()
            .profile()
            .actions
            .iter()
            .any(|a| {
                !a.disabled
                    && (a.short.kind == ActionType::VoiceDictate
                        || a.long.kind == ActionType::VoiceDictate)
            });
        let stt_missing = needs_stt && !firevibe_core::stt::authorized();

        if stt_missing {
            let st = firevibe_core::stt::auth_status();
            let l = self.l();
            return div()
                .flex()
                .flex_col()
                .gap(px(10.))
                .child(cards)
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(8.))
                        .w_full()
                        .px(px(12.))
                        .py(px(9.))
                        .rounded(px(R))
                        .border_1()
                        .border_color(c(WARN_LINE))
                        .bg(c(WARN_BG))
                        .child(div().text_color(c(WARN)).flex_none().child(icon("mic", 15.)))
                        .child(
                            div()
                                .flex_1()
                                .min_w(px(0.))
                                .flex()
                                .flex_col()
                                .child(
                                    div()
                                        .text_size(px(12.5))
                                        .font_weight(w(560.))
                                        .text_color(c(INK))
                                        .child(l.stt_unavailable()),
                                )
                                .child(
                                    div()
                                        .text_size(px(11.))
                                        .text_color(c(WARN))
                                        .mt(px(1.))
                                        .child(SharedString::from(l.stt_ask_hint(st))),
                                ),
                        )
                        .child(
                            mini2("stt-ask", l.request_perm()).h(px(32.)).on_click(cx.listener(
                                |this, _, _, cx| {
                                    std::thread::spawn(|| {
                                        let _ = firevibe_core::stt::request_auth();
                                    });
                                    this.toast(this.l().toast_requested());
                                    cx.notify();
                                },
                            )),
                        ),
                )
                .when_some(self.err.clone(), |d, e| d.child(self.err_bar(e, cx)))
                .into_any_element();
        }

        match &self.err {
            None => cards.into_any_element(),
            Some(e) => div()
                .flex()
                .flex_col()
                .gap(px(10.))
                .child(cards)
                .child(self.err_bar(e.clone(), cx))
                .into_any_element(),
        }
    }

    // ── 状态卡：按内容宽度排列，空间不足时自然换行 ──
    fn status_cards(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let l = self.l();
        let on = self.connected();
        let batt = self.battery();
        // 录音状态一次取完，别在 builder 链里反复上锁（见下面 when_some 处的说明）
        let rec_state = {
            let g = self.rt.recording.lock();
            g.as_ref().map(|r| (r.elapsed(), r.level()))
        };

        // 配对 + 电量
        let mut pair = div()
            .flex()
            .items_center()
            .flex_none()
            .bg(c(SURFACE))
            .border_1()
            .border_color(c(LINE))
            .gap(px(12.))
            .rounded(px(R))
            .px(px(14.))
            .py(px(9.))
            .shadow(sh1())
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(7.))
                    .text_size(px(12.5))
                    .child(
                        // 外圈浅色 + 内圈纯色的嵌套结构。
                        // 别用 border（往内收，7px 的点会只剩 1px 本色），
                        // 也别用 shadow spread（实测会盖住本体）。
                        div()
                            .size(px(16.))
                            .rounded(px(8.))
                            .flex_none()
                            .flex()
                            .items_center()
                            .justify_center()
                            .bg(if on { c(OK_SOFT) } else { gpui::transparent_black().into() })
                            .child(
                                div()
                                    .size(px(8.))
                                    .rounded(px(4.))
                                    .bg(if on { c(OK) } else { c(WARN) }),
                            ),
                    )
                    .child(
                        div()
                            .font_weight(w(560.))
                            .text_color(if on { c(INK) } else { c(WARN) })
                            .child(SharedString::from(if on {
                                l.paired()
                            } else {
                                l.unpaired()
                            })),
                    )
                    .child(div().ml(px(8.)).child(
                        ghost_btn("conn", if on { l.disconnect() } else { l.connect() }).on_click(
                            cx.listener(|this, _, _, cx| {
                                if this.connected() {
                                    this.rt.stop();
                                } else if let Err(e) = this.rt.start() {
                                    this.err = Some(format!("{e:#}"));
                                    this.err_sticky = true; // 用户主动连的，留住别自动消失
                                }
                                cx.notify();
                            }),
                        ),
                    ))
                    ,
            );
        // 电量读不到、而且是卡在蓝牙授权上，就给一句能点的提示 ——
        // 那个授权框只要还挂着没答复，蓝牙这条路就一直不通（见 core/src/battery.rs）
        if batt <= 0 && firevibe_core::battery::needs_bluetooth_permission() {
            pair = pair.child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(7.))
                    .pl(px(10.))
                    .border_l_1()
                    .border_color(c(LINE))
                    .text_size(px(12.5))
                    .child(
                        div()
                            .text_color(c(INK2))
                            .child(SharedString::from(l.battery_needs_bt())),
                    )
                    .child(mini2("open-bt", l.grant_access()).h(px(26.)).on_click(cx.listener(
                        |_, _, _, _| {
                            let _ = std::process::Command::new("open")
                                .arg("x-apple.systempreferences:com.apple.preference.security?Privacy_Bluetooth")
                                .spawn();
                        },
                    ))),
            );
        }
        if batt > 0 {
            pair = pair.child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(7.))
                    .pl(px(10.))
                    .border_l_1()
                    .border_color(c(LINE))
                    .text_size(px(12.5))
                    .child(battery_gauge(batt))
                    .child(
                        div()
                            .font_weight(w(590.))
                            .text_color(c(INK))
                            .child(SharedString::from(format!("{batt}%"))),
                    ),
            );
        }

        // 只保留一棵稳定的元素树。窗口缩放时由 flex-wrap 自然换行，避免跨断点
        // 替换整棵状态栏元素；每张卡片都 flex-none，宽度只由自身内容决定。
        let primary = div()
            .flex()
            .flex_wrap()
            .items_center()
            // 三张卡在默认英文窗口里刚好临界；8px 留出像素取整余量，输入卡便能
            // 留在首行并由 ml_auto 贴住最右侧。
            .gap(px(8.))
            .w_full()
            .child(pair)
            .child(self.loopback_card(cx))
            // 同行时由这个零基宽占位项吸收剩余空间，把输入卡推到最右；空间不足
            // 时只有输入卡换到下一行，它成为该行第一个元素，因此自然左对齐。
            .child(div().flex_1().min_w(px(0.)))
            .child(self.input_switch(cx));

        // ⚠️ 先把值取出来再建元素。写成
        // `.when(self.rt.recording.lock().is_some(), |d| d.child(self.recording_card()))`
        // 会死锁：判据里那个临时 guard 活到整条语句结束，闭包里又在同一线程
        // 上锁一次，而 parking_lot::Mutex 不可重入 —— UI 线程永久持锁，
        // 连 HID 线程都被拖死。录音状态另起一行，也不会继续挤压主状态行。
        div()
            .flex()
            .flex_col()
            .gap(px(10.))
            .child(primary)
            .when_some(rec_state, |d, (secs, lvl)| {
                d.child(Self::recording_card(secs, lvl, l))
            })
    }

    /// 录音中的状态卡。**只在本应用窗口里显示** —— 不弹任何系统界面。
    fn recording_card(secs: f32, lvl: f32, l: i18n::L) -> gpui::AnyElement {
        let mm = (secs as u32) / 60;
        let ss = (secs as u32) % 60;
        // 6 格小电平，跟着说话跳
        let lit = (lvl * 24.0).min(6.0) as usize;
        let mut meter = div().flex().items_end().gap(px(2.)).h(px(12.));
        for i in 0..6 {
            meter = meter.child(
                div()
                    .w(px(2.5))
                    .h(px(4. + i as f32 * 1.6))
                    .rounded(px(1.))
                    .bg(if i < lit { c(ERR) } else { c(LINE_STRONG) }),
            );
        }
        div()
            .flex()
            .items_center()
            .gap(px(9.))
            .flex_none()
            .rounded(px(R))
            .px(px(12.))
            .py(px(9.))
            .border_1()
            .border_color(c(ERR))
            .bg(c(SURFACE))
            .shadow(sh1())
            .child(div().size(px(8.)).rounded(px(4.)).flex_none().bg(c(ERR)))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .line_height(relative(1.25))
                    .child(
                        div()
                            .text_size(px(12.5))
                            .font_weight(w(580.))
                            .text_color(c(ERR))
                            .child(SharedString::from(l.recording_time(mm as u64, ss as u64))),
                    )
                    .child(
                        div()
                            .text_size(px(11.))
                            .text_color(c(INK3))
                            .child(l.recording_stop_hint()),
                    ),
            )
            .child(meter)
            .into_any_element()
    }

    /// 系统默认输入设备切换。放在状态行最右端。
    ///
    /// 为什么需要它：靠输入法做语音识别（豆包 / 闪电说这类）时，输入法听的是
    /// **系统默认输入**，所以得把它切到 BlackHole；用完又得切回真麦克风，
    /// 否则会议、系统听写全都听不到人声。省得每次去系统设置里翻。
    fn input_switch(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let l = self.l();
        let want = self.rt.cfg.read().voice.device.to_lowercase();
        let cur = self.audio_cur.clone();
        let name = cur.as_ref().map(|d| d.name.clone()).unwrap_or_else(|| "—".into());
        // 当前默认输入就是虚拟声卡 → 系统正在听遥控器
        let on_loopback = name.to_lowercase().contains(&want);

        let head = div()
            .id("input-switch")
            .flex()
            .w_full()
            .items_center()
            .gap(px(7.))
            .rounded(px(R))
            .px(px(12.))
            .py(px(9.))
            .border_1()
            .bg(c(SURFACE))
            .border_color(if on_loopback { c(ACCENT) } else { c(LINE) })
            .shadow(sh1())
            .cursor_pointer()
            .hover(|s| s.border_color(if on_loopback { c(ACCENT) } else { c(LINE_STRONG) }))
            .on_click(cx.listener(|this, _, _, cx| {
                if !this.just_dismissed() {
                    this.input_open = !this.input_open;
                    cx.notify();
                }
            }))
            .child(
                div()
                    .text_color(if on_loopback { c(ACCENT) } else { c(INK2) })
                    .flex_none()
                    .child(icon("mic", 15.)),
            )
            .child(
                div()
                    .flex_1()
                    .min_w(px(0.))
                    .flex()
                    .flex_col()
                    .line_height(relative(1.25))
                    .child(
                        div()
                            .overflow_hidden()
                            .text_ellipsis()
                            .text_size(px(12.5))
                            .font_weight(w(580.))
                            .text_color(if on_loopback { c(ACCENT_INK) } else { c(INK) })
                            .child(SharedString::from(name)),
                    )
                    .child(
                        div()
                            .text_size(px(11.))
                            .text_color(c(INK3))
                            .child(SharedString::from(l.system_input())),
                    ),
            )
            .child(
                div()
                    .flex_none()
                    .text_color(c(INK3))
                    .child(icon("chevron-down", 14.)),
            );

        // 内容宽度，不参与剩余空间分配；外层右槽负责把它贴到最右。
        // 设备名过长时最多占 220px，完整名称仍可在下拉菜单里查看。
        let mut wrap = div()
            .relative()
            .flex_none()
            .min_w(px(0.))
            .max_w(px(220.))
            .child(head);
        if self.input_open {
            let cur_id = cur.as_ref().map(|d| d.id);
            let mut menu = div()
                .absolute()
                .top(px(56.))
                .right(px(0.))
                .min_w(px(230.))
                .bg(c(SURFACE))
                .border_1()
                .border_color(c(LINE_STRONG))
                .rounded(px(10.))
                .shadow(sh3())
                .p(px(5.))
                .flex()
                .flex_col();
            for (i, d) in self.audio_list.iter().enumerate() {
                let id = d.id;
                let is_cur = Some(id) == cur_id;
                let is_lb = d.name.to_lowercase().contains(&want);
                menu = menu.child(
                    div()
                        .id(("indev", i))
                        .flex()
                        .items_center()
                        .gap(px(8.))
                        .px(px(9.))
                        .py(px(7.))
                        .rounded(px(7.))
                        .text_size(px(12.5))
                        .cursor_pointer()
                        .hover(|s| s.bg(c(MENU_HOVER)))
                        .text_color(if is_lb { c(ACCENT_INK) } else { c(INK) })
                        .child(
                            div()
                                .w(px(13.))
                                .flex_none()
                                .text_color(c(ACCENT))
                                .when(is_cur, |x| x.child(icon("check", 13.))),
                        )
                        .child(SharedString::from(d.name.clone()))
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.input_open = false;
                            // 丢后台，别在 update 里调 CoreAudio（会跑 run loop）；
                            // 也别在这里等结果 —— 阻塞 UI 线程还可能触发那个 RefCell panic。
                            // 结果由 pump 的定期刷新自然反映出来。
                            std::thread::spawn(move || {
                                let _ = audio::set_default_input(id);
                            });
                            this.audio_at = Instant::now() - Duration::from_secs(10);
                            cx.notify();
                        })),
                );
            }
            wrap = wrap.child(deferred(menu.on_mouse_down_out(cx.listener(
                |this, _, _, cx| {
                    this.dismiss_menus();
                    cx.notify();
                },
            ))));
        }
        wrap
    }

    /// 连接/权限出问题时的警示条。权限类问题不会自己好，所以常驻而不是 toast。
    /// 打开「配对新遥控器」弹窗：后台线程扫 HID 设备（list_hid 会跑 run loop，
    /// 不能在主线程/绘制期调），扫完通过 channel 回来。
    fn open_pairing(&mut self, cx: &mut Context<Self>) {
        self.pairing = true;
        self.pair_devices = None;
        let (tx, rx) = std::sync::mpsc::channel();
        self.pair_rx = Some(rx);
        std::thread::spawn(move || {
            let _ = tx.send(firevibe_core::device::list_hid());
        });
        cx.notify();
    }

    /// 选定一个设备作为遥控器：写进配置的 device_vid/pid，重连。
    fn pick_device(&mut self, vid: u16, pid: u16, cx: &mut Context<Self>) {
        {
            let mut c = self.rt.cfg.write();
            c.settings.device_vid = Some(format!("0x{vid:04x}"));
            c.settings.device_pid = Some(format!("0x{pid:04x}"));
            let _ = c.save();
        }
        firevibe_core::hidremap::set_ids(vid, pid);
        // 换了设备，旧电量作废 —— 不清的话界面会一直显示上一个遥控器的电量
        {
            let mut c = self.rt.cfg.write();
            c.settings.last_battery = None;
            let _ = c.save();
        }
        self.rt.status.battery.store(0, Ordering::Relaxed);
        firevibe_core::battery::forget();
        self.pairing = false;
        self.pair_devices = None;
        self.pair_rx = None;
        self.err = None;
        self.err_sticky = false;
        // 停掉旧连接再按新 ID 连
        self.rt.stop();
        match self.rt.start() {
            Ok(_) => self.toast(self.l().pair_ok()),
            Err(e) => {
                let m = format!("{e:#}");
                if !m.starts_with("HID_NOT_FOUND") {
                    self.err = Some(m);
                }
                self.toast(self.l().pair_saved());
            }
        }
        cx.notify();
    }

    /// 「配对新遥控器」弹窗
    fn pair_panel(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let l = self.l();
        let (cur_vid, cur_pid) = self.rt.cfg.read().device_ids();

        let mut body = div().flex().flex_col().gap(px(6.));
        match &self.pair_devices {
            None => {
                body = body.child(
                    div()
                        .py(px(20.))
                        .flex()
                        .items_center()
                        .justify_center()
                        .gap(px(8.))
                        .text_color(c(INK3))
                        .text_size(px(12.5))
                        .child(icon("loader-circle", 15.))
                        .child(SharedString::from(l.pair_scanning())),
                );
            }
            Some(devs) if devs.is_empty() => {
                body = body.child(
                    div().py(px(18.)).text_center().text_size(px(12.5)).text_color(c(INK2))
                        .child(SharedString::from(l.pair_none())),
                );
            }
            Some(devs) => {
                for d in devs {
                    let (vid, pid) = (d.vid, d.pid);
                    let is_cur = vid == cur_vid && pid == cur_pid;
                    // Fire TV 系 VID 0x0171 或名字带 remote 的高亮为「像遥控器」
                    let likely = vid == 0x0171 || d.label().to_lowercase().contains("remote");
                    let label = d.label();
                    let vendor = if d.vendor.is_empty() { String::new() } else { d.vendor.clone() };
                    let ids = d.ids();
                    body = body.child(
                        div()
                            .id(SharedString::from(format!("pd-{vid:04x}-{pid:04x}")))
                            .flex()
                            .items_center()
                            .gap(px(10.))
                            .px(px(12.))
                            .py(px(10.))
                            .rounded(px(9.))
                            .border_1()
                            .border_color(if is_cur { c(ACCENT) } else { c(LINE) })
                            .bg(if is_cur { c(ACCENT_SOFT) } else { c(SURFACE) })
                            .hover(|s| s.bg(c(LINE_SOFT)))
                            .cursor_pointer()
                            .on_click(cx.listener(move |this, _, _, cx| this.pick_device(vid, pid, cx)))
                            .child(
                                div()
                                    .flex_1()
                                    .min_w(px(0.))
                                    .flex()
                                    .flex_col()
                                    .gap(px(1.))
                                    .child(
                                        div().flex().items_center().gap(px(6.))
                                            .child(div().text_size(px(13.)).font_weight(w(560.)).text_color(c(INK)).child(SharedString::from(label)))
                                            .when(likely, |d| d.child(
                                                div().text_size(px(10.5)).text_color(c(ACCENT_INK)).child(SharedString::from(l.pair_likely()))
                                            )),
                                    )
                                    .child(div().text_size(px(11.)).text_color(c(INK3)).child(SharedString::from(
                                        if vendor.is_empty() { ids.clone() } else { format!("{ids}  ·  {vendor}") }
                                    ))),
                            )
                            .when(is_cur, |d| d.child(
                                div().flex_none().text_size(px(11.)).text_color(c(ACCENT_INK)).child(SharedString::from(l.pair_current()))
                            )),
                    );
                }
            }
        }

        crate::cards::overlay().child(
            div()
                .id("pair")
                .w(px(460.))
                .max_h(px(560.))
                .bg(c(SURFACE))
                .border_1()
                .border_color(c(LINE))
                .rounded(px(14.))
                .shadow(sh3())
                .px(px(20.))
                .py(px(18.))
                .flex()
                .flex_col()
                .gap(px(12.))
                .child(div().text_size(px(15.5)).font_weight(w(640.)).child(SharedString::from(l.pair_title())))
                .child(div().text_size(px(12.)).text_color(c(INK2)).line_height(relative(1.55)).child(SharedString::from(l.pair_hint())))
                .child(
                    div()
                        .id("pair-list")
                        .flex_1()
                        .min_h(px(0.))
                        .overflow_y_scroll()
                        .child(body),
                )
                .child(
                    div().flex().justify_between().items_center()
                        .child(
                            mini2("pair-rescan", l.pair_rescan()).on_click(cx.listener(|this, _, _, cx| this.open_pairing(cx))),
                        )
                        .child(
                            mini2("pair-close", l.cancel()).on_click(cx.listener(|this, _, _, cx| {
                                this.pairing = false;
                                this.pair_devices = None;
                                this.pair_rx = None;
                                cx.notify();
                            })),
                        ),
                ),
        )
    }

    fn err_bar(&self, msg: String, cx: &mut Context<Self>) -> impl IntoElement {
        // 按 core 给的 ASCII 前缀分类。别再用中文子串判断 ——
        // 之前错误消息里永远带「输入监控」，任何打不开设备都被误报成权限问题。
        let perm = msg.starts_with("HID_NOT_PERMITTED");
        let not_found = msg.starts_with("HID_NOT_FOUND");
        let l = self.l();
        div()
            .flex()
            .items_center()
            .gap(px(8.))
            .w_full()
            .px(px(12.))
            .py(px(9.))
            .rounded(px(R))
            .border_1()
            .border_color(c(WARN_LINE))
            .bg(c(WARN_BG))
            .child(div().text_color(c(WARN)).flex_none().child(icon("triangle-alert", 15.)))
            .child(
                div()
                    .flex_1()
                    .min_w(px(0.))
                    .flex()
                    .flex_col()
                    .child(
                        div()
                            .text_size(px(12.5))
                            .font_weight(w(560.))
                            .text_color(c(INK))
                            .child(SharedString::from(if perm {
                                l.hid_no_perm()
                            } else if not_found {
                                l.hid_not_connected()
                            } else {
                                l.hid_open_failed()
                            })),
                    )
                    .child(
                        div()
                            .text_size(px(11.))
                            .text_color(c(WARN))
                            .mt(px(1.))
                            .child(SharedString::from(if perm {
                                l.hid_perm_hint().to_string()
                            } else if not_found {
                                l.hid_not_found_hint().to_string()
                            } else {
                                msg.clone()
                            })),
                    ),
            )
            .child(
                // 两颗按钮同高（32）同一条轴，别一个 30 一个 28 错位
                div()
                    .flex()
                    .items_center()
                    .gap(px(6.))
                    .flex_none()
                    .when(not_found, |d| {
                        d.child(mini2("hid-retry", l.retry()).h(px(32.)).on_click(cx.listener(
                            |this, _, _, cx| {
                                this.err = None;
                                match this.rt.start() {
                                    Ok(_) => {
                                        this.err_sticky = false;
                                        this.toast(this.l().toast_connected());
                                    }
                                    Err(e) => {
                                        this.err = Some(format!("{e:#}"));
                                        this.err_sticky = true;
                                    }
                                }
                                cx.notify();
                            },
                        )))
                        .child(mini2("hid-repair", l.re_pair()).h(px(32.)).on_click(cx.listener(
                            |this, _, _, cx| this.open_pairing(cx),
                        )))
                    })
                    .when(perm, |d| {
                        d.child(
                            mini2("open-tcc", l.open_settings()).h(px(32.)).on_click(cx.listener(
                                |_, _, _, _| {
                                    let _ = std::process::Command::new("open")
                                        .arg("x-apple.systempreferences:com.apple.preference.security?Privacy_ListenEvent")
                                        .spawn();
                                },
                            )),
                        )
                        // 「勾了但不生效」的解法：清掉旧授权记录，让系统重新问一次。
                        // 根因是 ad-hoc 签名的 designated requirement 绑死 cdhash，
                        // 重建就失配；现在用证书签了，理论上不该再出现，留着兜底。
                        .child(
                            mini2("reset-tcc", l.reset_auth()).h(px(32.)).on_click(cx.listener(
                                |this, _, _, cx| {
                                    let out = std::process::Command::new("tccutil")
                                        .args(["reset", "ListenEvent", "com.tankxu.firevibe"])
                                        .output();
                                    match out {
                                        Ok(o) if o.status.success() => {
                                            this.toast(this.l().toast_reset_done())
                                        }
                                        _ => this.toast(this.l().toast_reset_failed()),
                                    }
                                    cx.notify();
                                },
                            )),
                        )
                    })
                    .child(icon_btn_px("err-x", "close", 32., 15., 8.).on_click(cx.listener(
                        |this, _, _, cx| {
                            this.err = None;
                            this.err_sticky = false;
                            cx.notify();
                        },
                    ))),
            )
    }

    /// BlackHole 卡：就绪 = 白卡打勾，未装 = 黄卡带安装按钮
    fn loopback_card(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let l = self.l();
        let ready = self.loopback.is_ready();
        let unknown = self.loopback.is_unknown();
        let name = self.rt.cfg.read().voice.device.clone();
        let name = if name.eq_ignore_ascii_case("blackhole") { "BlackHole 2ch".to_string() } else { name };

        let mut card = div()
            .flex()
            .flex_none()
            .items_center()
            .gap(px(7.))
            .rounded(px(R))
            .px(px(12.))
            .py(px(9.))
            .border_1()
            .shadow(sh1());
        card = if ready || unknown {
            card.bg(c(SURFACE)).border_color(c(LINE))
        } else {
            card.bg(c(WARN_BG)).border_color(c(WARN_LINE))
        };

        card.child(
            div()
                .flex_none()
                .text_color(if ready || unknown { c(INK2) } else { c(WARN) })
                .child(icon(
                    if unknown {
                        "loader-circle"
                    } else if ready {
                        "circle-check"
                    } else {
                        "triangle-alert"
                    },
                    15.,
                )),
        )
        .child(
            div()
                .flex_1()
                .min_w(px(0.))
                .flex()
                .flex_col()
                .line_height(relative(1.25))
                .child(
                    div()
                        .overflow_hidden()
                        .text_ellipsis()
                        .text_size(px(12.5))
                        .font_weight(w(580.))
                        .text_color(c(INK))
                        .child(SharedString::from(name)),
                )
                .child(
                    div()
                        .overflow_hidden()
                        .text_ellipsis()
                        .text_size(px(11.))
                        .text_color(if ready || unknown { c(INK3) } else { c(WARN) })
                        .child(SharedString::from(if unknown {
                            l.loopback_checking()
                        } else if ready {
                            l.loopback_ready()
                        } else {
                            l.loopback_missing()
                        })),
                ),
        )
        .when(ready, |d| {
            d.child(div().ml(px(6.)).flex_none().child(
                ghost_btn("voice-test", l.test()).on_click(cx.listener(|this, _, _, cx| {
                    this.voice_test = true;
                    this.test_frames0 = this.rt.status.audio_frames.load(Ordering::Relaxed);
                    cx.notify();
                })),
            ))
        })
        .when(!ready && !unknown, |d| {
            // 自己带着驱动就直接装（弹系统原生管理员授权框，密码不经过我们）；
            // 没带（没编过）才退回让用户装 BlackHole。
            let have = firevibe_core::audiodriver::bundled().is_some();
            d.child(div().ml(px(6.)).flex_none().child(
                install_btn("install-drv", l.install()).on_click(
                    cx.listener(move |this, _, _, cx| {
                        if !have {
                            let _ = std::process::Command::new("open")
                                .arg("https://existential.audio/blackhole/")
                                .spawn();
                            this.toast(this.l().toast_dl_opened());
                            cx.notify();
                            return;
                        }
                        // 先讲清楚要做什么，再走系统授权
                        this.install_confirm = true;
                        cx.notify();
                    }),
                ),
            ))
        })
    }

    // ── 方案选择 ──
    fn profile_block(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let l = self.l();
        let cfg = self.rt.cfg.read();
        let name = cfg.profile().name.clone();
        let n_keys = cfg.profile().actions.len();
        let n_prof = cfg.profiles.len();
        let names: Vec<String> = cfg.profiles.iter().map(|p| p.name.clone()).collect();
        let active = cfg.active;
        drop(cfg);

        let mut block = div()
            .flex()
            .flex_col()
            .relative()
            // 父级 items_start：方案名和下拉箭头只占内容宽度，不铺满整列
            .items_start()
            .child(section_lab(l.profile()).mb(px(1.)))
            .child(
                div()
                    .id("profile-pick")
                    .relative()
                    .flex()
                    .items_center()
                    .gap(px(7.))
                    .px(px(4.))
                    .pt(px(0.))
                    .pb(px(0.))
                    .ml(px(-4.))
                    .rounded(px(7.))
                    .cursor_pointer()
                    .hover(|s| s.bg(c(HOVER)))
                    .on_click(cx.listener(|this, _, _, cx| {
                        if !this.just_dismissed() {
                            this.profile_open = !this.profile_open;
                            cx.notify();
                        }
                    }))
                    .child(
                        div()
                            .text_size(px(23.))
                            .font_weight(w(640.))
                            .text_color(c(INK))
                            .child(SharedString::from(name)),
                    )
                    .child(
                        div()
                            .mt(px(4.))
                            .text_color(c(INK3))
                            .child(icon("chevron-down", 16.)),
                    ),
            )
            .child(
                div()
                    .text_size(px(12.))
                    .text_color(c(INK3))
                    .mt(px(-1.))
                    .child(SharedString::from(l.profile_meta(n_keys, n_prof))),
            );

        if self.profile_open {
            // 定位是相对整块（block）算的：分组标题 17.25+5 + 方案名行 39.5
            // ≈ 62，再留 4px 间隙
            let mut menu = div()
                .absolute()
                .top(px(66.))
                .left(px(0.))
                .min_w(px(180.))
                .bg(c(SURFACE))
                .border_1()
                .border_color(c(LINE_STRONG))
                .rounded(px(10.))
                .shadow(sh3())
                .p(px(5.))
                .flex()
                .flex_col();
            // 倒序显示：新建的在最上面，最早的那套（空白「默认」）沉到最下面。
            // 数组顺序仍然是创建顺序 —— active 存的是数组下标，别动它。
            for (i, n) in names.into_iter().enumerate().rev() {
                menu = menu.child(
                    div()
                        .id(("prof", i))
                        .flex()
                        .items_center()
                        .gap(px(8.))
                        .px(px(9.))
                        .py(px(7.))
                        .rounded(px(7.))
                        .text_size(px(12.5))
                        .cursor_pointer()
                        .hover(|s| s.bg(c(MENU_HOVER)))
                        .text_color(c(INK))
                        .child(
                            div()
                                .w(px(13.))
                                .text_color(c(ACCENT))
                                .when(i == active, |d| d.child(icon("check", 13.))),
                        )
                        .child(SharedString::from(n))
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.rt.cfg.write().active = i;
                            this.save();
                            this.profile_open = false;
                            cx.notify();
                        })),
                );
            }
            menu = menu
                .child(hline().my(px(4.)).mx(px(2.)))
                .child(
                    div()
                        .id("prof-rename")
                        .flex()
                        .items_center()
                        .gap(px(8.))
                        .px(px(9.))
                        .py(px(7.))
                        .rounded(px(7.))
                        .text_size(px(12.5))
                        .text_color(c(INK))
                        .cursor_pointer()
                        .hover(|s| s.bg(c(MENU_HOVER)))
                        .child(div().text_color(c(INK3)).child(icon("pencil", 13.)))
                        .child(SharedString::from(l.rename()))
                        .on_click(cx.listener(|this, _, window, cx| {
                            let cur = this.rt.cfg.read().profile().name.clone();
                            let input = crate::cards::new_line_input(&cur, window, cx);
                            // 回车直接保存。用组件的 PressEnter 而不是自己听 keydown ——
                            // 中文输入法组字时的 Return 被 macOS 的输入上下文吃掉用来上字，
                            // 压根不会走到按键绑定，所以不会误保存。
                            cx.subscribe(&input, |this, _, ev, cx| {
                                if matches!(ev, gpui_component::input::InputEvent::PressEnter { .. }) {
                                    this.commit_rename(cx);
                                }
                            })
                            .detach();
                            gpui::Focusable::focus_handle(&input, cx).focus(window);
                            this.renaming = Some(input);
                            this.profile_open = false;
                            cx.notify();
                        })),
                )
                .child(
                    div()
                        .id("prof-new")
                        .flex()
                        .items_center()
                        .gap(px(8.))
                        .px(px(9.))
                        .py(px(7.))
                        .rounded(px(7.))
                        .text_size(px(12.5))
                        .text_color(c(INK))
                        .cursor_pointer()
                        .hover(|s| s.bg(c(MENU_HOVER)))
                        .child(div().text_color(c(INK3)).child(icon("plus", 13.)))
                        .child(SharedString::from(l.new_profile()))
                        .on_click(cx.listener(|this, _, _, cx| {
                            let mut g = this.rt.cfg.write();
                            let n = g.profiles.len() + 1;
                            g.add_profile(format!("方案 {n}"));
                            g.active = g.profiles.len() - 1;
                            drop(g);
                            this.save();
                            this.profile_open = false;
                            cx.notify();
                        })),
                );
            // 同理，方案下拉要压在下面的「自定义操作」标题和卡片之上
            block = block.child(deferred(menu.on_mouse_down_out(cx.listener(
                |this, _, _, cx| {
                    this.dismiss_menus();
                    cx.notify();
                },
            ))));
        }
        block
    }
}

/// 电量小电池：外框 + 按百分比填充 + 右侧正极
fn battery_gauge(pct: i32) -> impl IntoElement {
    let fill = (pct.clamp(0, 100) as f32 / 100.) * 15.6;
    let col = if pct <= 15 { ERR } else if pct <= 30 { WARN } else { INK2 };
    div()
        .flex()
        .items_center()
        .gap(px(1.5))
        .child(
            div()
                .w(px(20.))
                .h(px(11.))
                .rounded(px(3.))
                .border(px(1.6))
                .border_color(c(col))
                .p(px(1.6))
                .child(
                    div()
                        .w(px(fill.max(1.)))
                        .h_full()
                        .rounded(px(1.4))
                        .bg(c(col)),
                ),
        )
        .child(div().w(px(2.2)).h(px(4.5)).rounded(px(1.1)).bg(c(col)))
}

impl FireVibe {
    /// 结束按住测试。多处调用（按钮松手 / 拖出去松手 / 关面板），做成幂等。
    fn stop_testing(&mut self, cx: &mut Context<Self>) {
        if self.testing {
            self.rt.set_talking(false);
            self.testing = false;
            cx.notify();
        }
        if self.dictating {
            let r = self.rt.set_dictating(false);
            self.dictating = false;
            if !r.is_empty() {
                self.toast(r);
            }
            cx.notify();
        }
    }



    /// 电量：HID 那条 `0x03` 报文要等设备主动上报，界面能空很久；
    /// 主动 GetReport 又被 BLE HOGP 拒（0xE00002F0）。所以走 CoreBluetooth
    /// 读标准电池服务 —— 实测能读到，见 core/src/battery.rs 的说明。
    fn poll_battery(&mut self) {
        if let Some(v) = firevibe_core::battery::last() {
            let was = self.rt.status.battery.swap(v, Ordering::Relaxed);
            if was != v {
                let mut g = self.rt.cfg.write();
                if g.settings.last_battery != Some(v) {
                    g.settings.last_battery = Some(v);
                    let _ = g.save();
                }
                drop(g);
                eprintln!("[batt] 蓝牙读到 {v}%");
            }
        }
    }

    /// 录快捷键：从 tap 会话里取结果。
    ///
    /// 走 tap 是因为被别的软件占用的组合键根本到不了窗口。tap 在录制期间还会
    /// 把那次按键吞掉，否则录「已占用的组合」会顺带把那个软件唤起来。
    fn poll_hotkey_grab(&mut self) {
        let Some(d) = &mut self.dialog else { return };
        let Some(g) = &d.grab else { return };
        if let Some(got) = g.take() {
            if got.key == "escape" {
                d.recording = false;
                d.grab = None;
                return;
            }
            d.key = got.key;
            d.mods = got.mods;
            d.recording = false;
            d.grab = None;
        } else if g.timed_out() {
            // 超时收摊 —— 别让 tap 一直吞用户的按键
            d.recording = false;
            d.grab = None;
        }
    }

    /// 提交方案改名。按钮和回车共用同一条路径。
    fn commit_rename(&mut self, cx: &mut Context<Self>) {
        let name = self
            .renaming
            .as_ref()
            .map(|i| i.read(cx).value().trim().to_string())
            .unwrap_or_default();
        if name.is_empty() {
            self.toast(self.l().toast_name_empty());
            return;
        }
        self.rt.cfg.write().profile_mut().name = name;
        self.save();
        self.renaming = None;
        cx.notify();
    }

    /// 方案改名
    fn rename_panel(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let Some(input) = self.renaming.clone() else {
            return div().into_any_element();
        };
        let l = self.l();
        crate::cards::overlay()
            .child(
                div()
                    .id("rn")
                    .w(px(360.))
                    .bg(c(SURFACE))
                    .border_1()
                    .border_color(c(LINE))
                    .rounded(px(14.))
                    .shadow(sh3())
                    .px(px(20.))
                    .py(px(18.))
                    .flex()
                    .flex_col()
                    .gap(px(12.))
                    .child(
                        div()
                            .text_size(px(15.))
                            .font_weight(w(640.))
                            .child(l.rename_title()),
                    )
                    .child(gpui_component::input::Input::new(&input))
                    .child(
                        div()
                            .flex()
                            .justify_end()
                            .gap(px(8.))
                            .child(mini2("rn-no", l.cancel()).on_click(cx.listener(
                                |this, _, _, cx| {
                                    this.renaming = None;
                                    cx.notify();
                                },
                            )))
                            .child(primary_btn("rn-ok", l.save()).on_click(cx.listener(
                                |this, _, _, cx| this.commit_rename(cx),
                            ))),
                    ),
            )
            .into_any_element()
    }

    /// 装虚拟声卡前的说明。系统授权框只有一句「osascript wants to make changes」，
    /// 什么都不解释就要密码是不合格的，先说清这是什么、为什么要装。
    /// 首次引导：配对 → 权限 → 装声卡，一次讲清，带路径和一键跳转。
    fn onboarding_panel(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let card_ready = self.loopback.is_ready();
        let paired = self.connected();
        let l = self.l();

        // 一步 = 彩色图标块 + 标题/说明 + 右侧（完成徽章 或 操作按钮）
        fn step(
            ic: &'static str,
            badge: (u32, u32, u32),
            title: &str,
            desc: &str,
            ready_label: &'static str,
            done: bool,
            action: Option<gpui::AnyElement>,
            last: bool,
        ) -> gpui::AnyElement {
            let right: gpui::AnyElement = if done {
                div()
                    .flex()
                    .items_center()
                    .gap(px(4.))
                    .flex_none()
                    .text_color(c(OK))
                    .text_size(px(11.5))
                    .font_weight(w(560.))
                    .child(icon("circle-check", 14.))
                    .child(SharedString::from(ready_label))
                    .into_any_element()
            } else if let Some(a) = action {
                a
            } else {
                div().into_any_element()
            };
            div()
                .flex()
                .items_center()
                .gap(px(13.))
                .px(px(16.))
                .py(px(14.))
                .when(!last, |d| d.border_b_1().border_color(c(LINE_SOFT)))
                .child(
                    div()
                        .size(px(34.))
                        .flex_none()
                        .rounded(px(9.))
                        .bg(grad(160., badge.0, badge.1))
                        .text_color(c(badge.2))
                        .shadow(sh1())
                        .flex()
                        .items_center()
                        .justify_center()
                        .child(icon(ic, 17.)),
                )
                .child(
                    div()
                        .max_w(px(340.))
                        .flex()
                        .flex_col()
                        .gap(px(2.))
                        .line_height(relative(1.5))
                        .child(div().text_size(px(13.5)).font_weight(w(600.)).text_color(c(INK)).child(SharedString::from(title.to_string())))
                        .child(div().text_size(px(12.)).text_color(c(INK2)).child(SharedString::from(desc.to_string()))),
                )
                // 弹性空白把右侧动作推到最右，文案不至于贴着按钮
                .child(div().flex_1().min_w(px(24.)))
                .child(div().flex_none().child(right))
                .into_any_element()
        }

        let open_url = |id: &'static str, label: &'static str, url: &'static str, cx: &mut Context<Self>| {
            mini2(id, label).on_click(cx.listener(move |_, _, _, _| {
                let _ = std::process::Command::new("open").arg(url).spawn();
            })).into_any_element()
        };

        crate::cards::overlay().child(
            div()
                .id("onb")
                .w(px(600.))
                .bg(c(SURFACE))
                .border_1()
                .border_color(c(LINE))
                .rounded(px(16.))
                .shadow(sh3())
                .flex()
                .flex_col()
                // 头部
                .child(
                    div()
                        .px(px(22.))
                        .pt(px(22.))
                        .pb(px(14.))
                        .flex()
                        .flex_col()
                        .gap(px(4.))
                        .child(div().text_size(px(18.)).font_weight(w(680.)).text_color(c(INK)).child(l.onb_title()))
                        .child(div().text_size(px(12.5)).text_color(c(INK2)).line_height(relative(1.5)).child(l.onb_subtitle())),
                )
                // 步骤列表（带边框分组）
                .child(
                    div()
                        .mx(px(22.))
                        .rounded(px(12.))
                        .border_1()
                        .border_color(c(LINE))
                        .bg(c(CODE_BG))
                        .overflow_hidden()
                        .child(step(
                            "tv", BADGE_DEFAULT,
                            l.onb_pair(),
                            l.onb_pair_desc(),
                            l.onb_ready(),
                            paired,
                            Some(open_url("onb-bt", l.onb_open_bt(), "x-apple.systempreferences:com.apple.preference.security?Privacy_Bluetooth", cx)),
                            false,
                        ))
                        .child(step(
                            "keyboard", BADGE_DEFAULT,
                            l.onb_im(),
                            l.onb_im_desc(),
                            l.onb_ready(),
                            false,
                            Some(open_url("onb-im", l.onb_open_settings(), "x-apple.systempreferences:com.apple.preference.security?Privacy_ListenEvent", cx)),
                            false,
                        ))
                        .child(step(
                            "mic", BADGE_DEFAULT,
                            l.onb_card(),
                            l.onb_card_desc(),
                            l.onb_ready(),
                            card_ready,
                            Some(
                                primary_btn("onb-inst", l.install()).on_click(cx.listener(|this, _, _, cx| {
                                    match firevibe_core::audiodriver::install() {
                                        Ok(()) => {
                                            this.rt.cfg.write().voice.device = firevibe_core::audiodriver::DEVICE_NAME.into();
                                            this.save();
                                            this.loopback = firevibe_core::voice::LoopbackStatus::Unknown;
                                            this.loopback_at = Instant::now() - Duration::from_secs(10);
                                            this.toast(this.l().toast_card_installed());
                                        }
                                        Err(e) => { let m = this.l().toast_install_failed(&e.to_string()); this.toast(m); }
                                    }
                                    cx.notify();
                                })).into_any_element(),
                            ),
                            false,
                        ))
                        .child(step(
                            "battery-full", BADGE_DEFAULT,
                            l.onb_bt(),
                            l.onb_bt_desc(),
                            l.onb_ready(),
                            false,
                            None,
                            true,
                        )),
                )
                // 底部
                .child(
                    div()
                        .px(px(22.))
                        .py(px(16.))
                        .flex()
                        .items_center()
                        .justify_between()
                        .child(div().text_size(px(11.5)).text_color(c(INK3)).child(l.onb_footer()))
                        .child(primary_btn("onb-done", l.onb_start()).on_click(cx.listener(|this, _, _, cx| {
                            this.onboarding = false;
                            this.rt.cfg.write().settings.onboarded = true;
                            this.save();
                            cx.notify();
                        }))),
                ),
        )
    }

    fn install_panel(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let dev = firevibe_core::audiodriver::DEVICE_NAME;
        let l = self.l();
        crate::cards::overlay().child(
            div()
                .id("inst")
                .w(px(440.))
                .bg(c(SURFACE))
                .border_1()
                .border_color(c(LINE))
                .rounded(px(14.))
                .shadow(sh3())
                .px(px(20.))
                .py(px(18.))
                .flex()
                .flex_col()
                .gap(px(14.))
                .child(
                    div()
                        .text_size(px(15.5))
                        .font_weight(w(640.))
                        .child(SharedString::from(l.install_title(dev))),
                )
                .child(
                    div()
                        .text_size(px(12.5))
                        .text_color(c(INK2))
                        .line_height(relative(1.6))
                        .child(l.install_body()),
                )
                .child(
                    div()
                        .flex()
                        .justify_end()
                        .gap(px(8.))
                        .child(mini2("inst-no", l.cancel()).on_click(cx.listener(
                            |this, _, _, cx| {
                                this.install_confirm = false;
                                cx.notify();
                            },
                        )))
                        .child(primary_btn("inst-go", l.install_continue()).on_click(cx.listener(
                            |this, _, _, cx| {
                                this.install_confirm = false;
                                match firevibe_core::audiodriver::install() {
                                    Ok(()) => {
                                        this.rt.cfg.write().voice.device =
                                            firevibe_core::audiodriver::DEVICE_NAME.into();
                                        this.save();
                                        this.loopback =
                                            firevibe_core::voice::LoopbackStatus::Unknown;
                                        this.loopback_at =
                                            Instant::now() - Duration::from_secs(10);
                                        this.toast(this.l().toast_installed_hint(firevibe_core::audiodriver::DEVICE_NAME));
                                    }
                                    Err(e) => { let m = this.l().toast_install_failed(&e.to_string()); this.toast(m); }
                                }
                                cx.notify();
                            },
                        ))),
                ),
        )
    }

    fn voice_test_panel(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let lvl = self.rt.level();
        let frames = self.rt.status.audio_frames.load(Ordering::Relaxed) - self.test_frames0;
        let mic_on = self.rt.status.mic_on.load(Ordering::Relaxed);
        let l = self.l();
        let cur_in = self
            .audio_cur
            .as_ref()
            .map(|d| d.name.clone())
            .unwrap_or_else(|| "—".into());
        let want = self.rt.cfg.read().voice.device.to_lowercase();
        let on_loopback = cur_in.to_lowercase().contains(&want);
        let ready = self.loopback.is_ready();
        let dicting = self.rt.dictating.lock().is_some();

        // 电平条：24 格
        let bars = (lvl * 60.0).min(1.0_f32.max(24.0)) as usize;
        let mut meter = div().flex().gap(px(3.)).h(px(28.)).items_end();
        for i in 0..24 {
            let lit = i < bars;
            let h = 8. + (i as f32 / 23.) * 20.;
            meter = meter.child(
                div()
                    .w(px(9.))
                    .h(px(h))
                    .rounded(px(2.))
                    .bg(if !lit {
                        c(LINE)
                    } else if i > 19 {
                        c(ERR)
                    } else if i > 15 {
                        c(WARN)
                    } else {
                        c(OK)
                    }),
            );
        }

        crate::cards::overlay()
            .child(
                div()
                    .id("vt")
                    .w(px(460.))
                    .bg(c(SURFACE))
                    .border_1()
                    .border_color(c(LINE))
                    .rounded(px(14.))
                    .shadow(sh3())
                    .child(
                        div()
                            .flex()
                            .items_start()
                            .gap(px(12.))
                            .px(px(18.))
                            .pt(px(16.))
                            .pb(px(14.))
                            .child(
                                div()
                                    .flex_1()
                                    .child(
                                        div()
                                            .text_size(px(15.))
                                            .font_weight(w(620.))
                                            .child(l.vt_title()),
                                    )
                                    .child(
                                        div()
                                            .text_size(px(11.5))
                                            .text_color(c(INK3))
                                            .mt(px(3.))
                                            .child(l.vt_hint()),
                                    ),
                            )
                            .child(icon_btn_sm("vt-x", "close").on_click(cx.listener(
                                |this, _, _, cx| {
                                    if this.testing {
                                        this.rt.set_talking(false);
                                        this.testing = false;
                                    }
                                    this.voice_test = false;
                                    cx.notify();
                                },
                            ))),
                    )
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .gap(px(14.))
                            .px(px(18.))
                            .pb(px(18.))
                            .child(
                                div()
                                    .flex()
                                    .flex_col()
                                    .gap(px(6.))
                                    .child(field_lab(l.vt_level()))
                                    .child(meter)
                                    .child(
                                        div()
                                            .text_size(px(11.5))
                                            .text_color(c(INK3))
                                            .child(SharedString::from(l.vt_level_line(lvl, frames, mic_on))),
                                    ),
                            )
                            .child(
                                div()
                                    .flex()
                                    .flex_col()
                                    .gap(px(4.))
                                    .child(field_lab(l.vt_default_input()))
                                    .child(
                                        div()
                                            .text_size(px(12.5))
                                            .font_weight(w(560.))
                                            .text_color(if on_loopback { c(ACCENT_INK) } else { c(INK) })
                                            .child(SharedString::from(cur_in)),
                                    )
                                    .child(
                                        div()
                                            .text_size(px(11.))
                                            .text_color(if on_loopback { c(INK3) } else { c(WARN) })
                                            .child(if on_loopback {
                                                l.vt_caption_on()
                                            } else {
                                                l.vt_caption_off()
                                            }),
                                    ),
                            )
                            .child(
                                div()
                                    .id("vt-hold")
                                    .h(px(46.))
                                    .rounded(px(10.))
                                    .flex()
                                    .items_center()
                                    .justify_center()
                                    .gap(px(7.))
                                    .cursor_pointer()
                                    .when(!ready, |d| d.opacity(0.5))
                                    .bg(if self.testing { c(ACCENT) } else { c(ACCENT_SOFT) })
                                    .text_color(if self.testing { c(SURFACE) } else { c(ACCENT_INK) })
                                    .font_weight(w(600.))
                                    .child(icon("mic", 16.))
                                    .child(SharedString::from(if self.testing {
                                        l.vt_recording()
                                    } else {
                                        l.vt_hold_talk()
                                    }))
                                    .on_mouse_down(
                                        gpui::MouseButton::Left,
                                        cx.listener(|this, _, _, cx| {
                                            if !this.loopback.is_ready() {
                                                this.toast(this.l().toast_card_not_ready());
                                                return;
                                            }
                                            this.test_frames0 = this
                                                .rt
                                                .status
                                                .audio_frames
                                                .load(Ordering::Relaxed);
                                            if this.rt.set_talking(true) {
                                                this.testing = true;
                                            } else {
                                                this.toast(this.l().toast_link_not_ready());
                                            }
                                            cx.notify();
                                        }),
                                    )
                                    // 松手必须挂在按钮自己身上 —— 面板遮罩带 occlude()，
                                    // root 上的 on_mouse_up 收不到；再加一个 _out
                                    // 兜住「按住后拖出按钮才松手」。
                                    .on_mouse_up(
                                        gpui::MouseButton::Left,
                                        cx.listener(|this, _, _, cx| {
                                            this.stop_testing(cx);
                                        }),
                                    )
                                    .on_mouse_up_out(
                                        gpui::MouseButton::Left,
                                        cx.listener(|this, _, _, cx| {
                                            this.stop_testing(cx);
                                        }),
                                    ),
                            )
                            // 听写走的是另一条路：不过虚拟声卡，直接把遥控器的 PCM
                            // 攒下来交给系统识别。放这里是为了不配遥控器也能验证。
                            .child(
                                div()
                                    .id("vt-dict")
                                    .h(px(46.))
                                    .rounded(px(10.))
                                    .flex()
                                    .items_center()
                                    .justify_center()
                                    .gap(px(7.))
                                    .cursor_pointer()
                                    .border_1()
                                    .border_color(c(LINE))
                                    .bg(if dicting { c(OK) } else { c(SURFACE) })
                                    .text_color(if dicting { c(SURFACE) } else { c(INK) })
                                    .font_weight(w(600.))
                                    .child(icon("case-sensitive", 16.))
                                    .child(SharedString::from(if dicting {
                                        l.vt_dictating()
                                    } else {
                                        l.vt_hold_dictate()
                                    }))
                                    .on_mouse_down(
                                        gpui::MouseButton::Left,
                                        cx.listener(|this, _, _, cx| {
                                            let r = this.rt.set_dictating(true);
                                            if r != "开始听写" {
                                                this.toast(r);
                                            }
                                            cx.notify();
                                        }),
                                    )
                                    .on_mouse_up(
                                        gpui::MouseButton::Left,
                                        cx.listener(|this, _, _, cx| {
                                            let _ = this.rt.set_dictating(false);
                                            cx.notify();
                                        }),
                                    )
                                    .on_mouse_up_out(
                                        gpui::MouseButton::Left,
                                        cx.listener(|this, _, _, cx| {
                                            let _ = this.rt.set_dictating(false);
                                            cx.notify();
                                        }),
                                    ),
                            )
                            .when_some(self.last_stt.clone(), |d, t| {
                                d.child(
                                    div()
                                        .flex()
                                        .flex_col()
                                        .gap(px(4.))
                                        .child(field_lab(l.vt_result()))
                                        .child(
                                            div()
                                                .text_size(px(12.5))
                                                .text_color(c(INK))
                                                .child(SharedString::from(t)),
                                        ),
                                )
                            }),
                    ),
            )
    }
}

impl Render for FireVibe {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.consume_boot(window, cx);
        // 顶栏与左侧遥控栏都固定不滑，只有右侧那列滚 —— 遥控栏自己也带滚动，
        // 但只在窗口矮到装不下它时才起作用。
        let body: gpui::AnyElement = match self.screen {
            Screen::Stats => div()
                .id("stats-scroll")
                .flex_1()
                .w_full()
                .min_h(px(0.))
                .overflow_y_scroll()
                .flex()
                .flex_col()
                .items_center()
                .px(px(32.))
                .pb(px(48.))
                .child(
                    div()
                        .w_full()
                        .max_w(px(720.))
                        .flex_none()
                        .child(self.stats_page(cx)),
                )
                .into_any_element(),
            Screen::Settings => div()
                .id("set-scroll")
                .flex_1()
                .w_full()
                .min_h(px(0.))
                .overflow_y_scroll()
                .flex()
                .flex_col()
                .items_center()
                .px(px(32.))
                .pb(px(48.))
                .child(
                    div()
                        .w_full()
                        .max_w(px(620.))
                        .flex_none()
                        .child(self.settings_page(cx)),
                )
                .into_any_element(),
            Screen::Main => div()
                .flex_1()
                .w_full()
                .min_h(px(0.))
                .flex()
                .justify_center()
                .px(px(32.))
                .child(
                    div()
                        .w_full()
                        .max_w(px(CONTENT_MAX_W))
                        .flex()
                        .flex_col()
                        .min_h(px(0.))
                        // 顶栏固定
                        .child(self.header(cx))
                        .child(
                            div()
                                .flex_1()
                                .min_h(px(0.))
                                .flex()
                                .gap(px(32.))
                                // 左：遥控器。固定不动，装不下才自己滑一点
                                .child(
                                    div()
                                        .id("left-col")
                                        .w(px(COL_LEFT_W))
                                        .flex_none()
                                        .min_h(px(0.))
                                        .overflow_y_scroll()
                                        .flex()
                                        .flex_col()
                                        .items_center()
                                        .gap(px(12.))
                                        .pt(px(34.))
                                        .pb(px(24.))
                                        .child(self.remote(cx))
                                        .child(
                                            div()
                                                .w(px(220.))
                                                .flex_none()
                                                .text_size(px(11.))
                                                .text_color(c(INK3))
                                                .text_center()
                                                .line_height(relative(1.55))
                                                .child(SharedString::from(self.l().remote_hint())),
                                        )
                                        .child(
                                            mini2_ico("go-stats", "chart-pie", self.l().stats_title())
                                                .on_click(cx.listener(|this, _, _, cx| {
                                                    this.screen = Screen::Stats;
                                                    cx.notify();
                                                })),
                                        ),
                                )
                                // 右：状态 / 方案 / 操作，这一列才滚
                                .child(
                                    div()
                                        .id("right-col")
                                        .flex_1()
                                        .min_w(px(0.))
                                        .min_h(px(0.))
                                        .overflow_y_scroll()
                                        .flex()
                                        .flex_col()
                                        .gap(px(22.))
                                        .pb(px(48.))
                                        .child(self.status_row(cx))
                                        .child(self.profile_block(cx))
                                        .child(self.action_section(cx)),
                                ),
                        ),
                )
                .into_any_element(),
        };

        let mut root = div()
            .id("root")
            .relative()
            .size_full()
            .flex()
            .flex_col()
            // 在窗口任意位置松手都要解除按下态 —— 否则按住拖出按钮再松开就卡住了
            .on_mouse_up(
                gpui::MouseButton::Left,
                cx.listener(|this, _, _, cx| {
                    let mut dirty = this.mouse_down.take().is_some();
                    if this.testing {
                        this.rt.set_talking(false);
                        this.testing = false;
                        dirty = true;
                    }
                    if dirty {
                        cx.notify();
                    }
                }),
            )
            .bg(c(BG))
            .text_color(c(INK))
            .text_size(px(14.))
            // gpui 默认行高是 φ(1.618)，设计稿是 CSS 的 1.5 —— 不改会一路累积
            // 垂直漂移（实测右栏比设计稿低 24px）
            .line_height(relative(1.5))
            // 顶部这条只管拖窗：红绿灯浮在这里。放在滚动区外面，往下滚也还能拖。
            .child(
                div()
                    .id("titlebar")
                    .w_full()
                    .h(px(TOPBAR_H))
                    .flex_none()
                    .window_control_area(gpui::WindowControlArea::Drag),
            )
            .child(body);

        if self.onboarding {
            root = root.child(self.onboarding_panel(cx));
        }
        if self.renaming.is_some() {
            root = root.child(self.rename_panel(cx));
        }
        if self.install_confirm {
            root = root.child(self.install_panel(cx));
        }
        if self.pairing {
            root = root.child(self.pair_panel(cx));
        }
        if self.voice_test {
            root = root.child(self.voice_test_panel(cx));
        }
        if self.dialog.is_some() {
            root = root.child(self.edit_dialog(cx));
        }
        if self.adding {
            root = root.child(self.add_panel(cx));
        }
        if let Some((msg, _)) = &self.toast {
            root = root.child(
                div()
                    .absolute()
                    .bottom(px(22.))
                    .left(px(0.))
                    .right(px(0.))
                    .flex()
                    .justify_center()
                    .child(
                        div()
                            .max_w(px(520.))
                            .px(px(14.))
                            .py(px(9.))
                            .rounded(px(10.))
                            .bg(c(INK))
                            .text_color(c(SURFACE))
                            .text_size(px(12.5))
                            .shadow(sh3())
                            .child(SharedString::from(msg.clone())),
                    ),
            );
        }
        root
    }
}

fn main() {
    let app = Application::new().with_assets(assets::Assets);
    // 从 Finder 再次打开应用时重新唤出窗口，并恢复普通应用的 Dock 图标。
    app.on_reopen(|cx| {
        firevibe_core::tray::show_from_tray();
        cx.activate(true);
    });
    app.run(|cx: &mut App| {
        gpui_component::init(cx);
        // HID 设备层映射是**进程外的系统状态** —— 我们退出了它还在，
        // 遥控器那颗键会一直是修饰键。退出时必须清掉。
        cx.on_app_quit(|_| {
            firevibe_core::hidremap::clear();
            async {}
        })
        .detach();

        // 应用菜单 + 退出 —— 之前顶部菜单栏空的、Cmd-Q 没绑，关窗又改成隐藏，
        // 结果 app 彻底退不掉。这里补一个 FireVibe 菜单，含「退出」并绑 Cmd-Q。
        cx.on_action(|_: &Quit, cx: &mut App| cx.quit());
        cx.bind_keys([gpui::KeyBinding::new("cmd-q", Quit, None)]);
        let ml = i18n::L(firevibe_core::config::Config::load().settings.lang);
        cx.set_menus(vec![Menu {
            name: "FireVibe".into(),
            items: vec![MenuItem::action(ml.menu_quit(), Quit)],
        }]);

        gpui_component::Theme::change(gpui_component::ThemeMode::Light, None, cx);
        // 输入框聚焦时的边框走 theme.ring，默认是近黑色，一圈粗黑边太重。
        // 换成我们的强调色，和别处的选中态一致。
        gpui_component::Theme::global_mut(cx).ring = c(ACCENT).into();

        // 补齐 macOS 那套 emacs 光标快捷键。gpui-component 只绑了 ctrl-a/ctrl-e
        //（行首行尾），而 **ctrl-f 被它绑成了代码编辑器的搜索**，所以按下去光标不动；
        // ctrl-b/n/p/d/k 压根没绑。系统里这些是 AppKit 提供的默认行为，
        // gpui 不会转交给它，只能自己补。后注册的绑定优先，所以能盖掉 Search。
        {
            use gpui::KeyBinding;
            use gpui_component::input::{
                Delete, DeleteToEndOfLine, MoveDown, MoveLeft, MoveRight, MoveUp,
            };
            const IN: Option<&str> = Some("Input");
            cx.bind_keys([
                KeyBinding::new("ctrl-f", MoveRight, IN),
                KeyBinding::new("ctrl-b", MoveLeft, IN),
                KeyBinding::new("ctrl-n", MoveDown, IN),
                KeyBinding::new("ctrl-p", MoveUp, IN),
                KeyBinding::new("ctrl-d", Delete, IN),
                KeyBinding::new("ctrl-k", DeleteToEndOfLine, IN),
            ]);
        }
        // 自检时可以用 FIREVIBE_WIN=1100x1400 把窗口撑高，一次截全长页面
        let (ww, wh) = std::env::var("FIREVIBE_WIN")
            .ok()
            .and_then(|v| {
                let (a, b) = v.split_once('x')?;
                Some((a.trim().parse::<f32>().ok()?, b.trim().parse::<f32>().ok()?))
            })
            .unwrap_or((1100., 820.));
        let bounds = Bounds::centered(None, size(px(ww), px(wh)), cx);
        cx.open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(bounds)),
                // 系统标题栏整条隐掉，内容直接顶到窗口边；红绿灯浮在我们自己
                // 画的那条顶栏上，并按顶栏高度垂直居中 —— 现在 macOS 应用的标配长相。
                titlebar: Some(gpui::TitlebarOptions {
                    title: Some("FireVibe".into()),
                    appears_transparent: true,
                    traffic_light_position: Some(gpui::point(px(19.), px(TOPBAR_H / 2. - 6.))),
                }),
                window_min_size: Some(size(px(880.), px(560.))),
                ..Default::default()
            },
            |window, cx| {
                // 点红叉 = 隐藏到后台，**不**走 GPUI 默认关闭 —— 默认会 drop 掉最后一个
                // 窗口，触发主线程死循环（关窗后卡死、要 force quit 的根因）。返回 false
                // 阻止关闭，改为纯菜单栏模式：隐藏窗口和 Dock 图标，只保留 tray。
                // tray 的“显示窗口”会恢复窗口与 Dock；Cmd-Q 才真正退出
                //（退出会跑 on_app_quit 清掉 hidremap）。
                window.on_window_should_close(cx, |_window, _cx| {
                    firevibe_core::tray::hide_to_tray();
                    false
                });
                let view = cx.new(FireVibe::new);
                cx.new(|cx| Root::new(view, window, cx))
            },
        )
        .unwrap();
        cx.activate(true);
        // 右上角菜单栏状态项（图标 + 显示/退出菜单）—— 关窗隐藏后也能从这里操作
        let tl = i18n::L(firevibe_core::config::Config::load().settings.lang);
        firevibe_core::tray::install(
            include_bytes!("../assets/tray/tray@2x.png"),
            tl.tray_show(),
            tl.tray_quit(),
        );
        // 让窗口背景可拖（含整个 header）—— gpui 的 window_control_area/start_window_move
        // 在 mac 上是空实现，只能靠 NSWindow.movableByWindowBackground。
        firevibe_core::tray::make_windows_draggable();
    });
}
