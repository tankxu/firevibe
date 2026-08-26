//! `--probe-all`：换一款遥控器时的**唯一入口**。
//!
//! 一条命令把该查的、该问的、该看的全走一遍，每一步的结果直接喂给下一步：
//! 选出来的设备标识写进配置并被后面所有步骤复用，认到的键位当场落盘，
//! 报文观察的结论决定最后告诉用户「哪些功能能用」。
//!
//! 全程还会把每一行都抄进 `~/Downloads/FireVibe 适配报告 <时间>.txt`，
//! 末尾带一段机器可读的 JSON —— 后续脚本直接读那段，不用去解析中文。
//!
//! ⚠️ 只发 `0xF2` 这一个已知命令的两个已知取值（0x01 开麦 / 0x00 关麦）。
//! **不对不明设备做 opcode 盲扫** —— 描述符里那个 10 字节的 `0xF3` OUTPUT
//! 语义未知，一个字都不碰（BLE GATT 那边就有过 WIPE 命令的先例）。

use firevibe_core::{
    config::Config,
    device::HidDev,
    keys::Key,
    layout::Slot,
    runtime::{Event, Runtime},
};
use std::io::Write;
use std::sync::atomic::Ordering;
use std::sync::mpsc::Receiver;
use std::time::{Duration, Instant};

/// 原厂 Fire TV Alexa Voice Remote (3rd Gen) 0x0171/0x0421 的报告描述符。
/// 平替常把它原样抄过去，所以「一致」只说明抄了，**不说明固件实现了语音**。
const STOCK_DESC: &str = "05010906a1010507850195037508150025ff190029ff8100c0050c0901a1018502950275101500269c0219002a9c028100c006ff000900a10185f095507508150025ff810085f1950375080900810285f2950175080900910285f3950a75080900910285ef95037508150025ff190029ff09018100c0050c0901a101850305010906a102050609201500266400750895018102c0c0";

// ─────────────────────────── 报告收集 ───────────────────────────

/// 边打印边攒。最后整份落盘 —— 让用户回传一个文件，比让他复制终端可靠得多。
struct Report {
    lines: Vec<String>,
    facts: Vec<(String, String)>,
}

impl Report {
    fn new() -> Self {
        Report { lines: Vec::new(), facts: Vec::new() }
    }
    /// 打印并记下
    fn say(&mut self, s: impl AsRef<str>) {
        let s = s.as_ref();
        println!("{s}");
        self.lines.push(s.to_string());
    }
    /// 只记不打（进度条那种刷屏的东西不进文件）
    fn note(&mut self, s: impl Into<String>) {
        self.lines.push(s.into());
    }
    /// 机器可读的一条事实，最后汇成 JSON 给后续脚本用
    fn fact(&mut self, k: &str, v: impl Into<String>) {
        self.facts.push((k.to_string(), v.into()));
    }
    fn save(&self) -> Option<std::path::PathBuf> {
        let dir = dirs_downloads()?;
        let path = dir.join(format!("FireVibe 适配报告 {}.txt", stamp()));
        let mut body = self.lines.join("\n");
        body.push_str("\n\n--- 机器可读（后续脚本读这段）---\n{\n");
        for (i, (k, v)) in self.facts.iter().enumerate() {
            let comma = if i + 1 == self.facts.len() { "" } else { "," };
            body.push_str(&format!("  {:?}: {:?}{comma}\n", k, v));
        }
        body.push_str("}\n");
        std::fs::write(&path, body).ok()?;
        Some(path)
    }
}

fn dirs_downloads() -> Option<std::path::PathBuf> {
    let home = std::env::var_os("HOME")?;
    let p = std::path::PathBuf::from(home).join("Downloads");
    p.is_dir().then_some(p)
}

/// 文件名用的时间戳。进程里没有 chrono，直接问 `date`——
/// 这命令在任何 macOS 上都有，比自己算历法可靠。
fn stamp() -> String {
    std::process::Command::new("date")
        .arg("+%Y-%m-%d %H-%M-%S")
        .output()
        .ok()
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .map(|s| s.trim().to_string())
        .unwrap_or_else(|| "未知时间".into())
}

fn ask(prompt: &str) -> String {
    print!("{prompt}");
    let _ = std::io::stdout().flush();
    let mut s = String::new();
    let _ = std::io::stdin().read_line(&mut s);
    s.trim().to_string()
}

/// 等用户回车。每一步开始前都停一下 —— 不然提示还没看完测试就跑过去了，
/// 上一版就是这么把「按住 12 秒」测成「只收到 1 条报文」的。
fn wait_enter(prompt: &str) {
    print!("{prompt}");
    let _ = std::io::stdout().flush();
    let mut s = String::new();
    let _ = std::io::stdin().read_line(&mut s);
}

// ─────────────────────── HID 报告描述符解析 ───────────────────────

#[derive(Debug, Clone, Copy, PartialEq)]
enum Dir {
    In,
    Out,
    Feature,
}

impl Dir {
    fn zh(self) -> &'static str {
        match self {
            Dir::In => "INPUT  设备→主机",
            Dir::Out => "OUTPUT 主机→设备",
            Dir::Feature => "FEATURE",
        }
    }
}

/// 一条 report：id、方向、载荷字节数
#[derive(Debug, Clone, Copy)]
struct RepDef {
    id: u8,
    dir: Dir,
    bytes: u32,
}

/// 够用的短条目解析器：只跟 Report ID / Report Size / Report Count，
/// 遇到 INPUT/OUTPUT/FEATURE 就结算一条。长条目（0xFE）直接跳过。
fn parse_desc(d: &[u8]) -> Vec<RepDef> {
    let mut out: Vec<RepDef> = Vec::new();
    let (mut rid, mut size, mut count) = (0u8, 0u32, 0u32);
    let mut i = 0usize;
    while i < d.len() {
        let b = d[i];
        if b == 0xFE {
            // 长条目：下一个字节是数据长度
            let n = *d.get(i + 1).unwrap_or(&0) as usize;
            i += 3 + n;
            continue;
        }
        let len = match b & 0x03 {
            0 => 0,
            1 => 1,
            2 => 2,
            _ => 4,
        };
        let val: u32 = (0..len).fold(0u32, |acc, k| {
            acc | (*d.get(i + 1 + k).unwrap_or(&0) as u32) << (8 * k as u32)
        });
        match b & 0xFC {
            0x84 => rid = val as u8,       // Report ID
            0x74 => size = val,            // Report Size
            0x94 => count = val,           // Report Count
            0x80 => push(&mut out, rid, Dir::In, size, count), // INPUT
            0x90 => push(&mut out, rid, Dir::Out, size, count), // OUTPUT
            0xB0 => push(&mut out, rid, Dir::Feature, size, count), // FEATURE
            _ => {}
        }
        i += 1 + len;
    }
    out
}

fn push(out: &mut Vec<RepDef>, id: u8, dir: Dir, size: u32, count: u32) {
    let bytes = (size * count).div_ceil(8);
    // 同一条 report 可能由多段拼成，累加
    if let Some(e) = out.iter_mut().find(|e| e.id == id && e.dir == dir) {
        e.bytes += bytes;
    } else {
        out.push(RepDef { id, dir, bytes });
    }
}

fn rid_zh(rid: u8) -> &'static str {
    match rid {
        0x01 => "键盘键",
        0x02 => "多媒体键",
        0x03 => "电量",
        0xF0 => "音频流（我们认的那条）",
        0xF2 => "开关麦命令",
        0xEF | 0xF1 | 0xF3 => "厂商私有",
        _ => "未知",
    }
}

// ───────────────────────────── 主流程 ─────────────────────────────

pub fn run() -> anyhow::Result<()> {
    // Ctrl-C 兜底：麦克风那步会临时把麦克风键映射成右⌥，中途退出必须清掉，
    // 否则那颗键会一直是右⌥（进程外的系统状态）。SIGKILL 拦不住，但 Ctrl-C 能。
    #[cfg(unix)]
    unsafe {
        extern "C" fn on_sigint(_: i32) {
            firevibe_core::hidremap::clear();
            std::process::exit(130);
        }
        libc::signal(libc::SIGINT, on_sigint as libc::sighandler_t);
    }
    let mut r = Report::new();
    r.say("");
    r.say("╭─ 配一个遥控器 ───────────────────────────────────────╮");
    r.say("│ 帮你在 FireVibe 里用上一个遥控器 —— 原装 Fire TV 遥控器，│");
    r.say("│ 或外观相同的副厂遥控器都行。跟着提示走就成。           │");
    r.say("│                                                      │");
    r.say("│ 会带你：                                             │");
    r.say("│   ① 认出遥控器  ② 挨个记下按键                        │");
    r.say("│   ③ 试语音能不能用  ④ 看电量能不能读                  │");
    r.say("│ 最后问你要不要把这套写进 FireVibe（不写也行）。       │");
    r.say("│                                                      │");
    r.say("│ 约 2~3 分钟。哪儿不对，末尾的报告文件发回来就能帮你查。│");
    r.say("╰─────────────────────────────────────────────────────╯");
    r.fact("时间", stamp());

    step0_env(&mut r);
    let Some(dev) = step1_pick(&mut r) else {
        finish(&mut r);
        return Ok(());
    };

    // ⚠️ 适配是纯硬件探测，**不读也不写 app 的配置**。这里只在内存里造一份临时
    // 配置，把选中的设备标识塞进去用来打开设备 —— 从不落盘。认到的键、结论都只
    // 进报告；最后单独问一句要不要写进 FireVibe 配置。
    let mut cfg = Config::default();
    cfg.settings.device_vid = Some(format!("0x{:04x}", dev.vid));
    cfg.settings.device_pid = Some(format!("0x{:04x}", dev.pid));
    // ⚠️ 别用独占(seize)打开 —— 那需要 root，普通用户会 privilege violation(0xE00002C1)。
    // Spotlight 改在测麦克风那步用临时按键重映射抑制（见 step4_mic）。
    let (rt, rx) = Runtime::new(cfg);
    // hidremap 要按这台设备匹配（transient 全局状态，非配置文件）
    firevibe_core::hidremap::set_ids(dev.vid, dev.pid);
    if let Err(e) = rt.start() {
        r.say(format!("\n打不开这台设备：{e:#}"));
        r.say("常见原因：①「输入监控」没给终端 ②遥控器蓝牙断了 ③FireVibe 还开着占用设备");
        r.fact("打开设备", format!("失败: {e}"));
        finish(&mut r);
        return Ok(());
    }
    rt.set_learn(true); // 只上报，不执行动作 —— 免得认键时真把 Mac 操作了
    rt.raw_all.store(true, Ordering::Relaxed);
    std::thread::sleep(Duration::from_millis(400));
    r.say(format!("\n✓ 已连上「{}」{}", dev.label(), dev.ids()));

    step2_descriptor(&mut r, &rt);
    let learned = step3_keys(&mut r, &rt, &rx);
    let mic = step4_mic(&mut r, &rt, &rx);
    step6_battery(&mut r, &rt, &rx);
    step7_verdict(&mut r, mic, learned.len());

    rt.raw_all.store(false, Ordering::Relaxed);
    rt.stop();

    // 探测本身零副作用。要不要把「设备 + 认到的键位」写进 FireVibe 配置，单独问。
    maybe_apply(&mut r, &dev, &learned);

    finish(&mut r);
    Ok(())
}

fn finish(r: &mut Report) {
    // 兜底：万一麦克风步骤中途退出，别把临时按键映射留在系统里
    firevibe_core::hidremap::clear();
    match r.save() {
        Some(p) => {
            let line = format!("\n报告已存到：{}", p.display());
            println!("{line}");
            println!("把这个文件发回来就行（终端里的内容不用复制）。\n");
        }
        None => println!("\n（写报告文件失败，把上面整段终端内容复制发回来）\n"),
    }
}

// ── 第 0 步：环境 ──
fn step0_env(r: &mut Report) {
    r.say("\n───── 先检查一下环境 ─────");
    let ver = std::process::Command::new("sw_vers")
        .arg("-productVersion")
        .output()
        .ok()
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .map(|s| s.trim().to_string())
        .unwrap_or_default();
    r.say(format!("  macOS {ver}"));
    r.fact("macOS", ver);

    // FireVibe 开着会占住设备，两个进程抢同一个 HID 句柄必然有一个失败
    let running = std::process::Command::new("pgrep")
        .args(["-f", "FireVibe.app"])
        .output()
        .map(|o| !o.stdout.is_empty())
        .unwrap_or(false);
    if running {
        r.say("  ⚠ FireVibe 正开着 —— 它占着遥控器，这次探测会打不开设备。");
        r.say("    请先完全退出 FireVibe（菜单栏 › 退出）再重跑本命令。");
        wait_enter("    退出好了按回车继续（或 Ctrl-C 退出）… ");
    } else {
        r.say("  ✓ FireVibe 没在跑，设备没人占");
    }
    r.fact("FireVibe在跑", running.to_string());
}

// ── 第 1 步：选设备，写进配置 ──
fn step1_pick(r: &mut Report) -> Option<HidDev> {
    r.say("\n───── ① 认出你的遥控器 ─────");
    r.say("  先确认遥控器已经在 系统设置 › 蓝牙 里连上了。");
    let devs = firevibe_core::device::list_hid();
    if devs.is_empty() {
        r.say("  ✗ 一个 HID 设备都没扫到。先去蓝牙里连上遥控器再重跑。");
        r.fact("设备", "没扫到");
        return None;
    }
    r.say("");
    for (i, d) in devs.iter().enumerate() {
        // Fire TV 系都是 VID 0x0171；名字里带 Remote 的也高亮一下
        let hint = if d.vid == 0x0171 || d.label().to_lowercase().contains("remote") {
            "  ← 像是遥控器"
        } else {
            ""
        };
        let vendor = if d.vendor.is_empty() { "厂商未知" } else { &d.vendor };
        r.say(format!(
            "  {:>2}) {:<28} {}  {vendor}{hint}",
            i + 1,
            d.label(),
            d.ids()
        ));
    }
    // 只有一个像遥控器的就默认它，回车即可
    let guess = devs
        .iter()
        .position(|d| d.vid == 0x0171 || d.label().to_lowercase().contains("remote"));
    let prompt = match guess {
        Some(g) => format!("\n  输入序号（直接回车＝选 {}）: ", g + 1),
        None => "\n  输入序号: ".to_string(),
    };
    let ans = ask(&prompt);
    let idx = if ans.is_empty() { guess } else { ans.parse::<usize>().ok().map(|n| n.wrapping_sub(1)) };
    let Some(d) = idx.and_then(|i| devs.get(i)).cloned() else {
        r.say("  没选到设备，结束。");
        r.fact("设备", "未选择");
        return None;
    };
    r.note(format!("  选了：{} {}", d.label(), d.ids()));
    // 只记录，不动配置 —— 适配全程零副作用
    r.fact("VID", format!("0x{:04x}", d.vid));
    r.fact("PID", format!("0x{:04x}", d.pid));
    r.fact("产品名", d.label());
    Some(d)
}

// ── 第 2 步：描述符 ──
fn step2_descriptor(r: &mut Report, rt: &Runtime) {
    // 屏幕上只说人话；十六进制、report 明细都进报告文件（r.note / r.fact），
    // 那些是给开发者排障看的，不该在向导里刷屏吓人。
    r.say("\n───── 读取遥控器信息 ─────");
    let d = rt.descriptor.lock().clone();
    let hex: String = d.iter().map(|b| format!("{b:02x}")).collect();
    r.fact("描述符", hex.clone());
    r.note(format!("  报告描述符 {} 字节: {hex}", d.len()));

    let same = hex == STOCK_DESC;
    r.fact("与原厂描述符一致", same.to_string());
    if same {
        r.say("  ✓ 和原装 Fire TV 遥控器一模一样 —— 大概率能用（后面几步会实测确认）。");
    } else {
        r.say("  · 和原装遥控器不同，是另一套硬件 —— 后面几步看看哪些功能能用。");
    }

    let defs = parse_desc(&d);
    if !defs.is_empty() {
        r.note("  声明的 report：");
        for e in &defs {
            r.note(format!("    0x{:02X}  {:<16} {:>3} 字节   {}", e.id, e.dir.zh(), e.bytes, rid_zh(e.id)));
        }
        let has_audio = defs.iter().any(|e| e.id == 0xF0 && e.dir == Dir::In);
        let cmd = defs.iter().find(|e| e.id == 0xF2 && e.dir == Dir::Out);
        r.fact("声明0xF0音频", has_audio.to_string());
        r.fact("0xF2命令载荷字节", cmd.map(|e| e.bytes.to_string()).unwrap_or_else(|| "无".into()));
    }
}

// ── 第 3 步：逐键认键，直接落盘 ──
fn step3_keys(r: &mut Report, rt: &Runtime, rx: &Receiver<Event>) -> Vec<(Slot, Key)> {
    r.say("\n───── ② 记下每个按键 ─────");
    r.say("  会依次点名，按下即记录并前进；遥控器上没有的键等 6 秒自动跳过。");
    r.say("  ⚠️ 认到的键**只先记在报告里**，最后会单独问你要不要写进 FireVibe 配置。");
    let mut learned: Vec<(Slot, Key)> = Vec::new();
    if ask("  现在做吗？(回车＝做，n＝跳过): ").to_lowercase() == "n" {
        r.say("  跳过认键。");
        r.fact("认键", "跳过");
        return learned;
    }
    println!("  {}", "─".repeat(56));

    let mut raws: Vec<String> = Vec::new();
    for (i, slot) in Slot::ALL.into_iter().enumerate() {
        print!("  [{:>2}/{}] 请按「{}」… ", i + 1, Slot::ALL.len(), slot.label());
        let _ = std::io::stdout().flush();
        let t0 = Instant::now();
        let mut done = false;
        let mut seen_raw: Vec<String> = Vec::new();
        while t0.elapsed() < Duration::from_secs(6) {
            match rx.recv_timeout(Duration::from_millis(100)) {
                Ok(Event::Raw { report_id, data }) => {
                    let h: String =
                        data.iter().take(8).map(|b| format!("{b:02X} ")).collect();
                    seen_raw.push(format!("0x{report_id:02X} {}", h.trim_end()));
                }
                Ok(Event::Learned(k)) => {
                    println!("记下 {k}");
                    learned.push((slot, k));
                    done = true;
                    break;
                }
                _ => {}
            }
        }
        if !done {
            println!("跳过");
        }
        if let Some(first) = seen_raw.first() {
            raws.push(format!("{}: {first}", slot.label()));
        }
        while rx.try_recv().is_ok() {}
    }
    println!("  {}", "─".repeat(56));
    r.say(format!("  记下 {} / {} 个键（暂存报告，未写配置）。", learned.len(), Slot::ALL.len()));
    r.fact("认到的键数", learned.len().to_string());
    r.fact(
        "键位映射",
        learned.iter().map(|(s, k)| format!("{}={}", s.id(), k.id())).collect::<Vec<_>>().join(" "),
    );
    if !raws.is_empty() {
        r.note("  各键原始报文：");
        for l in &raws {
            r.note(format!("    {l}"));
        }
    }
    learned
}

/// 探测结束后单独问：要不要把「设备 + 认到的键位」写进 FireVibe 配置。
/// **这是全流程唯一会碰配置文件的地方** —— 适配探测本身零副作用。
fn maybe_apply(r: &mut Report, dev: &HidDev, learned: &[(Slot, Key)]) {
    r.say("\n───── 写进 FireVibe 配置？─────");
    if learned.is_empty() {
        r.say("  这次没认到键（跳过或没测），不写配置。");
        r.say("  （设备标识、探测结果都只在报告里，没动你的配置。）");
        r.fact("写入配置", "否（无键位）");
        return;
    }
    r.say(format!(
        "  可把「设备 {} + 认到的 {} 个键位」写进 FireVibe 配置。",
        dev.ids(),
        learned.len()
    ));
    r.say("  会覆盖当前方案里这些键的物理映射（按键**动作**保留），并设成这台设备。");
    if ask("  写进去吗？(y＝写 / 回车＝不写): ").to_lowercase() != "y" {
        r.say("  没写。你的配置保持原样。");
        r.fact("写入配置", "否");
        return;
    }
    let mut cfg = Config::load();
    cfg.settings.device_vid = Some(format!("0x{:04x}", dev.vid));
    cfg.settings.device_pid = Some(format!("0x{:04x}", dev.pid));
    for (slot, k) in learned {
        cfg.set_slot(*slot, *k);
    }
    match cfg.save() {
        Ok(_) => {
            r.say("  ✓ 已写进配置。重开 FireVibe 生效。");
            r.fact("写入配置", "是");
        }
        Err(e) => {
            r.say(format!("  ✗ 写失败：{e}"));
            r.fact("写入配置", format!("失败: {e}"));
        }
    }
}

/// 一段观察窗：把窗口内每条报文的原始字节打出来（每种 report id 只详打前 6 条），
/// 返回 (报文总数, 音频帧增量, 各 report id 计数)
fn watch(
    r: &mut Report,
    rt: &Runtime,
    rx: &Receiver<Event>,
    secs: u64,
    t0: Instant,
) -> (u64, u64, Vec<(u8, u64)>) {
    let a0 = rt.status.audio_frames.load(Ordering::Relaxed);
    let mut shown: std::collections::HashMap<u8, u32> = std::collections::HashMap::new();
    let mut total = 0u64;
    let end = Instant::now() + Duration::from_secs(secs);
    while Instant::now() < end {
        while let Ok(ev) = rx.try_recv() {
            if let Event::Log(_) = &ev {
                // 命令回执（keepalive 每秒一条）—— 丢掉，不然刷屏；出没出流看帧数就行
                continue;
            }
            if let Event::Raw { report_id, data } = ev {
                total += 1;
                let c = shown.entry(report_id).or_insert(0);
                *c += 1;
                if *c <= 6 {
                    let hex: String =
                        data.iter().take(12).map(|b| format!("{b:02X} ")).collect();
                    let more =
                        if data.len() > 12 { format!("… 共 {} 字节", data.len()) } else { String::new() };
                    r.say(format!(
                        "    [{:5.1}s] 0x{report_id:02X}  {}{more}",
                        t0.elapsed().as_secs_f32(),
                        hex.trim_end()
                    ));
                } else if *c == 7 {
                    r.say(format!("    [{:5.1}s] 0x{report_id:02X}  （同类报文后面不再逐条打）", t0.elapsed().as_secs_f32()));
                }
            }
        }
        std::thread::sleep(Duration::from_millis(25));
    }
    let audio = rt.status.audio_frames.load(Ordering::Relaxed) - a0;
    let counts: Vec<(u8, u64)> = rt.seen_rids.lock().iter().map(|(k, v)| (*k, *v)).collect();
    (total, audio, counts)
}

// ── 第 4 步：按住麦克风键（被动） ──
/// 一轮：先发一条命令（可选）→ 静观几秒 → 让人按住 8 秒。
/// 返回这一轮收到的音频帧数。
fn hold_round(
    r: &mut Report,
    rt: &Runtime,
    rx: &Receiver<Event>,
    name: &str,
    pre: Option<Vec<u8>>,
    idle_watch: u64,
) -> u64 {
    if let Some(b) = pre {
        let hex: String = b.iter().map(|x| format!("{x:02X} ")).collect();
        r.say(format!("  先发 {}", hex.trim_end()));
        rt.send_report(b);
        std::thread::sleep(Duration::from_millis(600));
    }
    let t0 = Instant::now();
    let a0 = rt.status.audio_frames.load(Ordering::Relaxed);
    if idle_watch > 0 {
        // 不按键先看几秒：能自己吐流就说明麦克风是「热」的
        r.say(format!("  先不要按键，静观 {idle_watch} 秒…"));
        watch(r, rt, rx, idle_watch, t0);
        let idle = rt.status.audio_frames.load(Ordering::Relaxed) - a0;
        r.say(format!(
            "  没按键的这 {idle_watch} 秒收到 {idle} 帧{}",
            if idle > 0 { "（麦克风是热的，这很费电）" } else { "" }
        ));
    }
    let b0 = rt.status.audio_frames.load(Ordering::Relaxed);
    wait_enter(&format!("  【{name}】按回车，然后**立刻按住麦克风键说话**，8 秒… "));
    while rx.try_recv().is_ok() {}
    watch(r, rt, rx, 8, t0);
    println!("  ↑ 松开麦克风键");
    watch(r, rt, rx, 2, t0);
    let got = rt.status.audio_frames.load(Ordering::Relaxed) - b0;
    r.say(format!("  【{name}】按住期间收到 {got} 帧"));
    got
}

/// 麦克风探测 —— **不用碰遥控器**。
///
/// 走和界面「按住说话」完全一样的路径：先 `ensure_voice()` 建虚拟声卡 sink，
/// 再 `set_talking(true)`（内部 `set_passing` + 置 `mic_on`，读线程随即发
/// `MIC_ON [F2,01,01]` 并每秒 keepalive）。麦克风是「热」的 —— 命令一发就出流，
/// 不需要物理按住那颗键。
/// ⚠️ 早先这一步要求用户按住麦克风键，是因为当时没建 sink：
/// `push_pcm` 在 `passing=false` 时直接丢弃，看着就像「不按就没声音」。
///
/// 返回 (收到的 0xF0 帧数, 峰值电平, 见到的报文 id)
fn step4_mic(r: &mut Report, rt: &Runtime, rx: &Receiver<Event>) -> (u64, f32, Vec<u8>) {
    r.say("\n───── ③ 试试语音输入 ─────");
    r.say("  主机发开麦命令，看遥控器吐不吐音频 —— **不用你按遥控器**。");
    // 诊断期间别让 app 逻辑自己发关麦干扰
    rt.auto_mic_off.store(false, Ordering::Relaxed);

    // 建语音链路（虚拟声卡 sink）。没有它 push_pcm 会被丢弃，等于测不到。
    if let Err(e) = rt.ensure_voice() {
        r.say(format!("  ⚠ 语音链路建不起来：{e:#}"));
        r.say("    多半是虚拟声卡「FireVibe Mic」没装 —— 在 FireVibe 里装一下再来。");
        r.fact("语音", format!("链路失败: {e}"));
        rt.auto_mic_off.store(true, Ordering::Relaxed);
        return (0, 0.0, Vec::new());
    }

    rt.seen_rids.lock().clear();
    while rx.try_recv().is_ok() {}
    let t0 = Instant::now();
    let a0 = rt.status.audio_frames.load(Ordering::Relaxed);

    if !rt.set_talking(true) {
        r.say("  ⚠ 开不了麦（语音链路没就绪）");
        rt.auto_mic_off.store(true, Ordering::Relaxed);
        return (0, 0.0, Vec::new());
    }
    r.say("  已开麦，收 8 秒…（想验电平就对着遥控器说话，不说也能看出流没流）");

    // 收 8 秒，顺带记峰值电平
    let mut peak = 0.0f32;
    for _ in 0..8 {
        watch(r, rt, rx, 1, t0);
        let lv = rt.level();
        if lv > peak {
            peak = lv;
        }
    }
    let frames = rt.status.audio_frames.load(Ordering::Relaxed) - a0;

    // 收尾：关麦（热麦克风一直开着很费电）
    rt.set_talking(false);
    std::thread::sleep(Duration::from_millis(300));
    rt.send_report(vec![0xF2, 0x00]);
    std::thread::sleep(Duration::from_millis(300));
    rt.auto_mic_off.store(true, Ordering::Relaxed);

    let ids: Vec<u8> = rt.seen_rids.lock().keys().copied().collect();
    r.say(format!("  收到 {frames} 帧 0xF0 音频，峰值电平 {peak:.3}"));
    if !ids.is_empty() {
        r.say("  期间见到的报文类型：");
        for rid in &ids {
            r.say(format!("    0x{rid:02X}  {}", rid_zh(*rid)));
        }
    }
    r.fact("麦克风_帧数", frames.to_string());
    r.fact("麦克风_峰值电平", format!("{peak:.3}"));
    r.fact(
        "报文id",
        ids.iter().map(|k| format!("0x{k:02X}")).collect::<Vec<_>>().join(" "),
    );
    (frames, peak, ids)
}

fn step6_battery(r: &mut Report, rt: &Runtime, rx: &Receiver<Event>) {
    r.say("\n───── ④ 看看电量 ─────");
    r.say("  电量报文（0x03）是设备想发才发的，不一定等得到，等不到也不影响别的功能。");
    rt.seen_rids.lock().clear();
    while rx.try_recv().is_ok() {}
    let t0 = Instant::now();
    let (_, _, counts) = watch(r, rt, rx, 5, t0);
    let got = counts.iter().any(|(k, _)| *k == 0x03);
    r.say(if got { "  ✓ 收到电量报文" } else { "  · 这 5 秒没等到（正常，它发得很稀）" });
    r.fact("收到电量报文", got.to_string());
}

// ── 结论 ──
fn step7_verdict(r: &mut Report, mic: (u64, f32, Vec<u8>), key_count: usize) {
    let (frames, peak, ids) = mic;
    r.say("\n───── 结论 ─────");

    if frames > 0 {
        r.say(format!("  ✓ 语音可用：开麦后收到 {frames} 帧 0xF0 音频（峰值电平 {peak:.3}），"));
        r.say("    和原装 Fire TV 遥控器同一条通路，麦克风能在 FireVibe 里用。");
        if peak < 0.005 {
            r.say("    （电平很低是正常的 —— 刚才没人对着它说话，出流本身就说明通路是好的。）");
        }
        r.fact("语音", "可用");
    } else if ids.iter().any(|id| *id >= 0xE0 && *id != 0xF2) {
        let list: Vec<String> = ids.iter().filter(|id| **id >= 0xE0).map(|id| format!("0x{id:02X}")).collect();
        r.say("  ? 0xF0 上没有音频，但见到了别的厂商私有报文：");
        r.say(format!("    {}", list.join(" ")));
        r.say("    语音也许走了别的 report id —— 把这份报告发回来能进一步定位。");
        r.fact("语音", "待定位");
    } else {
        r.say("  ✗ 语音用不了：开麦命令发出去了，但一帧 0xF0 都没收到。");
        r.say("    这台在 HID 层大概没实现 Amazon 那套语音通路。");
        r.fact("语音", "不可用");
    }

    if key_count > 0 {
        r.say(format!("  ✓ 按键：认到 {key_count} 个（末尾可选择写进配置）。"));
    } else {
        r.say("  · 按键：没认到（跳过了，或设备选错）。");
    }

    r.say("");
    r.say("  下一步：把报告文件发回来。末尾那段 JSON 是给后续脚本读的，不用你整理。");
}
