//! 无界面版：在做 GUI 之前把引擎跑通，也用于排障。
mod map;
mod sniff;
use firevibe_core::{
    config::{Config, VoiceMode},
    runtime::{Event, Runtime},
    voice::list_output_devices,
};
use std::sync::atomic::Ordering;
use std::time::{Duration, Instant};

/// 装个 SIGINT 处理器。没引 ctrlc crate，直接用 libc。
fn ctrlc_like<F: Fn() + Send + 'static>(f: F) -> anyhow::Result<()> {
    static mut HOOK: Option<Box<dyn Fn() + Send>> = None;
    unsafe {
        HOOK = Some(Box::new(f));
        extern "C" fn on_sigint(_: i32) {
            // SAFETY: HOOK 在装处理器之前就写好了，之后只读
            #[allow(static_mut_refs)]
            unsafe {
                if let Some(h) = HOOK.as_ref() {
                    h();
                }
            }
        }
        libc::signal(libc::SIGINT, on_sigint as libc::sighandler_t);
    }
    Ok(())
}

/// 强制开麦 + 送流，实时看电平。**不看按键怎么配的** ——
/// 用来把「音频链路」和「按键动作链路」分开定位。
///
/// `hold` 给了的话同时按住那个快捷键，模拟「按住说话 + 触发第三方工具」。
fn run_mic(hold: Option<String>) -> anyhow::Result<()> {
    let cfg = Config::load();
    let dev = cfg.voice.device.clone();
    let (rt, rx) = Runtime::new(cfg);
    let rt = std::sync::Arc::new(rt);
    rt.start()?;
    rt.ensure_voice()?;
    let sink = rt.voice_sink().ok_or_else(|| anyhow::anyhow!("语音链路没建起来"))?;

    println!("虚拟声卡: {} @ {} Hz / {} 声道", sink.device_name, sink.out_rate, sink.out_channels);
    println!("匹配前缀: {dev}");
    // 强制开麦 + 送流 + 把系统默认输入切到虚拟声卡。
    // 少了最后一步的话，靠系统默认输入做识别的工具（豆包这类）压根听不到。
    let before = firevibe_core::audio::default_input();
    rt.set_talking(true);
    std::thread::sleep(Duration::from_millis(300));
    println!(
        "系统默认输入: {} → {}",
        before.map(|d| d.name).unwrap_or_else(|| "?".into()),
        firevibe_core::audio::default_input().map(|d| d.name).unwrap_or_else(|| "?".into())
    );
    if let Some(k) = &hold {
        match rt.inj.key_down(k, &[]) {
            Ok(_) => println!("已按住快捷键 {k}（退出时松开）"),
            Err(e) => println!("按住 {k} 失败: {e}"),
        }
    }
    println!("\n对着遥控器说话。电平应该跟着动；不动就是音频没进来。Ctrl-C 退出。\n");

    // Ctrl-C 时把设备还原回去
    {
        let rt2 = rt.clone();
        let _ = ctrlc_like(move || {
            rt2.set_talking(false);
            std::thread::sleep(Duration::from_millis(600));
            rt2.restore_input();
            std::thread::sleep(Duration::from_millis(400));
            std::process::exit(0);
        });
    }

    let t0 = std::time::Instant::now();
    let mut last = std::time::Instant::now();
    loop {
        while let Ok(ev) = rx.try_recv() {
            if let Event::Log(s) = ev {
                println!("  · {s}");
            }
        }
        if last.elapsed() >= Duration::from_millis(400) {
            last = std::time::Instant::now();
            let s = &rt.status;
            let frames = s.audio_frames.load(Ordering::Relaxed);
            let lvl = sink.level();
            let bars = (lvl * 60.0).min(40.0) as usize;
            print!(
                "\r  {:>4}s  麦克风 {}  已收 {:>6} 帧  丢 {:>4}  电平 {:.4} |{:<40}|",
                t0.elapsed().as_secs(),
                if s.mic_on.load(Ordering::Relaxed) { "开" } else { "关" },
                frames,
                sink.dropped(),
                lvl,
                "#".repeat(bars)
            );
            use std::io::Write;
            let _ = std::io::stdout().flush();
        }
        std::thread::sleep(Duration::from_millis(30));
    }
}

/// 直接执行某个位置配置好的动作，绕开 HID 按键。用来单独验动作链路。
fn run_action_once(spec: &str) -> anyhow::Result<()> {
    use firevibe_core::layout::Slot;
    let (slot_id, trig) = spec.split_once(':').unwrap_or((spec, "short"));
    let slot = Slot::ALL
        .into_iter()
        .find(|s| s.id() == slot_id)
        .ok_or_else(|| anyhow::anyhow!("不认识的位置 {slot_id}；可用：{:?}",
            Slot::ALL.iter().map(|s| s.id()).collect::<Vec<_>>()))?;
    let long = trig == "long";

    let cfg = Config::load();
    let act = cfg.profile().action(slot, long);
    let (rt, _rx) = Runtime::new(cfg);
    let rt = std::sync::Arc::new(rt);
    rt.ensure_voice().ok(); // 语音动作要有 sink，起不来也继续
    let Some(act) = act else {
        println!("{} 的{}按没配动作", slot.label(), if long { "长" } else { "短" });
        return Ok(());
    };
    println!("执行 {} {}按 → {}", slot.label(), if long { "长" } else { "短" }, act.describe());
    let r = rt.run_action(&act, true);
    println!("  按下 → {r}");
    if long {
        println!("  保持 2 秒…");
        std::thread::sleep(Duration::from_secs(2));
        let r2 = rt.run_action(&act, false);
        println!("  松开 → {r2}");
    }
    Ok(())
}

/// 诊断：系统把遥控器的键变成了什么事件。
///
/// 用来定位「按麦克风键弹 Spotlight」这类问题 —— 麦克风键的 usage 是
/// Consumer 0x0221 (AC Search)，macOS 自己就把它当搜索键。
///
/// **只打印非字符键**（功能/媒体键区、修饰键、systemDefined），
/// 你打的字一个都不记录。
fn run_tap() -> anyhow::Result<()> {
    use firevibe_core::tap;
    println!("正在监听系统事件（只看非字符键）。按遥控器上的键试试，Ctrl-C 退出。");
    println!("需要「辅助功能」权限；没有的话下面会直接报错。\n");
    println!("{:<16} {:<8} {:<10} {:<9} {:<7} {}", "事件类型", "键码", "flags", "来源", "键盘类型", "备注");
    let _t = tap::spawn(
        &[tap::EV_KEY_DOWN, tap::EV_KEY_UP, tap::EV_FLAGS_CHANGED, tap::EV_SYSTEM_DEFINED],
        true, // 只看不拦
        Box::new(|_| false),
        Some(Box::new(|ev: tap::Ev| {
            // FIREVIBE_TAP_ALL=1 才不过滤（排障用，会看到所有按键）
            let all = std::env::var("FIREVIBE_TAP_ALL").is_ok();
            if !all && !tap::is_non_character(ev) {
                return;
            }
            let name = match ev.kind {
                tap::EV_KEY_DOWN => "keyDown",
                tap::EV_KEY_UP => "keyUp",
                tap::EV_FLAGS_CHANGED => "flagsChanged",
                tap::EV_SYSTEM_DEFINED => "systemDefined",
                other => {
                    println!("(未知类型 {other})");
                    return;
                }
            };
            let note = if ev.kind == tap::EV_SYSTEM_DEFINED {
                format!("NX 键码 {} {}", ev.code, if ev.nx_down { "按下" } else { "松开" })
            } else {
                String::new()
            };
            let src = if ev.pid == 0 { "硬件".to_string() } else { format!("pid {}", ev.pid) };
            println!(
                "{name:<16} 0x{:<6X} 0x{:<8X} {src:<9} {:<7} {note}",
                ev.code, ev.flags, ev.kb_type
            );
        })),
    )?;
    loop {
        std::thread::sleep(Duration::from_secs(1));
    }
}

fn main() -> anyhow::Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let has = |f: &str| args.iter().any(|a| a == f);
    let val = |f: &str| {
        args.iter()
            .position(|a| a == f)
            .and_then(|i| args.get(i + 1))
            .cloned()
    };

    if has("--help") || has("-h") {
        println!(
            "firevibe-cli [选项]\n\
             \x20 --list-devices     列出音频输出设备\n\
             \x20 --exclusive        独占打开设备（系统收不到原始按键）\n\
             \x20 --device <前缀>    覆盖输出设备\n\
             \x20 --mode gate|always 覆盖送流方式\n\
             \x20 --gain <倍数>      覆盖增益\n\
             \x20 --no-voice         只测按键\n\
             \x20 --descriptor       打印 HID 报告描述符后退出\n\
             \x20 --map              按键测绘：逐个记录每个物理键的真实 HID usage\n\
             \x20 --sniff            原始 report 嗅探：打印每一条报文（含 vendor 0xEF/0xF1）\n\
             \x20 --tap              看系统把遥控器按键翻译成了什么事件（只打印非字符键）\n\
             \x20 --mic              强制开麦并送流进虚拟声卡，实时看电平。\n\
             \x20                    只测音频链路，完全不看按键怎么配的。\n\
             \x20 --mic --hold <键>  同时按住一个快捷键（模拟「按住说话 + 触发第三方工具」）\n\
             \x20 --run <位置>:<short|long>  直接执行某个位置配置好的动作，比如 mic:long\n\
             \x20 --inputs           列出可用输入设备并标出当前默认\n\
             \x20 --set-input <前缀> 切换系统默认输入设备"
        );
        return Ok(());
    }

    if has("--map") {
        return map::run();
    }

    if has("--tap") {
        return run_tap();
    }

    // 对照实验：真键盘按住修饰键 vs 我们合成的，同一个 tap 里逐字段比。
    // 「合成的按住 Option 和键盘是不是一回事」只能这么验，不能靠推理。
    // 用真遥控器测：只盯着系统的修饰键状态，把每次变化按时间打出来。
    // 前面那些对照都是 CLI 自己合成的，真实路径（遥控器→HID→长按判定→注入）
    // 一次都没量过 —— 这个就是量它的。
    if has("--watch-mods") {
        return run_watch_mods();
    }

    if has("--modcmp") {
        return run_modcmp(val("--modcmp").filter(|s| !s.starts_with("--")));
    }

    if has("--mic") {
        return run_mic(val("--hold"));
    }

    if let Some(spec) = val("--run") {
        return run_action_once(&spec);
    }

    // 排障用：把一段文字打进当前前台 app，验证注入链路
    if let Some(t) = val("--type") {
        std::thread::sleep(Duration::from_millis(600));
        let inj = firevibe_core::inject::new_injector();
        eprintln!("辅助功能可用 = {}", inj.available());
        inj.type_text(&t)?;
        eprintln!("已发出 {} 字符 / {} UTF-16 单元", t.chars().count(), t.encode_utf16().count());
        return Ok(());
    }

    // 虚拟声卡自检：往输出端写测试音，从输入端录回来。
    // 豆包这类工具听的就是输入端 —— 这里录不到，它也听不到。
    if has("--loopback-test") {
        let dev = val("--loopback-test")
            .filter(|s| !s.starts_with("--"))
            .unwrap_or_else(|| "blackhole".into());
        println!("往 {dev:?} 的输出端发 1.5s 的 440Hz，同时从它的输入端录…");
        let (sent, heard) = firevibe_core::voice::loopback_selftest(&dev, 1.5)?;
        println!("  发出 RMS = {sent:.4}");
        println!("  录回 RMS = {heard:.4}");
        if heard > sent * 0.2 {
            println!("  ✓ 虚拟声卡通了 —— 听系统默认输入的工具能收到遥控器的声音");
        } else if heard > 0.0005 {
            println!("  ⚠ 有信号但很弱（{:.0}%），检查声卡增益/声道", heard / sent * 100.0);
        } else {
            println!("  ✗ 输入端没有信号 —— 虚拟声卡没把输出回环到输入");
        }
        return Ok(());
    }

    // 给第三方语音工具做的「它到底认不认」实验：把系统默认输入固定成虚拟声卡，
    // 同时持续往里灌一段响亮的测试音。不用碰遥控器 —— 打开豆包/闪电说看电平动不动，
    // 动了说明它跟随系统默认输入，不动说明它绑死在别的设备上。
    if has("--feed-tone") {
        let dev = val("--feed-tone")
            .filter(|s| !s.starts_with("--"))
            .unwrap_or_else(|| "blackhole".into());
        // 输入和输出可能是两个设备（拆开是为了绕过 VPIO 的回声消除），
        // 所以要固定的输入设备名单独给：--pin-input <名字>；不给就不动默认输入。
        let pin = val("--pin-input").filter(|s| !s.starts_with("--"));
        let prev = firevibe_core::audio::default_input();
        let mut target = None;
        if let Some(pin) = &pin {
            target = firevibe_core::audio::input_devices()
                .into_iter()
                .find(|d| d.name.to_lowercase().contains(&pin.to_lowercase()));
            match &target {
                Some(t) => {
                    println!("默认输入：{} → {}",
                        prev.as_ref().map(|d| d.name.as_str()).unwrap_or("?"), t.name);
                    firevibe_core::audio::set_default_input_and_wait(t.id, Duration::from_millis(500));
                }
                None => println!("没找到输入设备 {pin:?}，不改默认输入"),
            }
        }
        let _ = &target;

        // Ctrl-C 时一定要把设备还原回去，不然用户的麦克风就留在虚拟声卡上了
        let prev_id = if pin.is_some() { prev.as_ref().map(|d| d.id) } else { None };
        let restored = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        {
            let restored = restored.clone();
            let _ = ctrlc_like(move || {
                if !restored.swap(true, std::sync::atomic::Ordering::SeqCst) {
                    if let Some(id) = prev_id {
                        let _ = firevibe_core::audio::set_default_input(id);
                        eprintln!("\n默认输入已还原");
                    }
                }
                std::process::exit(0);
            });
        }

        let sink = firevibe_core::voice::VoiceSink::start(&dev, 1.0)?;
        sink.set_passing(true);
        println!("\n正在往 {dev} 灌 440Hz 测试音。");
        println!("现在打开你的第三方语音输入工具，看它的电平动不动 —— Ctrl-C 结束并还原设备。\n");
        let mut buf = vec![0i16; firevibe_core::voice::OPUS_FRAME];
        let mut phase = 0.0f32;
        let step = std::f32::consts::TAU * 440.0 / firevibe_core::voice::OPUS_RATE as f32;
        let mut tick = 0u32;
        loop {
            for s in buf.iter_mut() {
                *s = (phase.sin() * 0.35 * i16::MAX as f32) as i16;
                phase += step;
            }
            sink.push_pcm(&buf);
            std::thread::sleep(Duration::from_millis(20));
            tick += 1;
            if tick % 50 == 0 {
                print!("\r  已灌 {}s   ", tick / 50);
                use std::io::Write;
                let _ = std::io::stdout().flush();
            }
        }
    }

    // 只读地加载一次配置并打印摘要。加载会跑迁移，所以也用来验迁移。
    if has("--config") {
        let c = firevibe_core::config::Config::load();
        println!("配置：{}", firevibe_core::config::config_path().display());
        println!("  schema = {}", c.schema);
        println!("  虚拟声卡 = {:?}", c.voice.device);
        println!("  硬件层映射 = {:?}", c.mic_remap_key());
        println!("  方案：");
        for (i, p) in c.profiles.iter().enumerate() {
            let mark = if i == c.active { " ← 当前" } else { "" };
            println!("    [{i}] {:<10} {} 个动作{mark}", p.name, p.actions.len());
        }
        return Ok(());
    }

    // 列出所有 HID 设备。判断「另一款外观相同的遥控器能不能用」时先看这个：
    // 我们是按 VID/PID 打开设备的，标识不一样就完全看不到它。
    if has("--hid-list") {
        let (want_vid, want_pid) = firevibe_core::config::Config::load().device_ids();
        let api = hidapi::HidApi::new()?;
        println!("{:<8} {:<8}  {:<28} {}", "VID", "PID", "产品名", "厂商");
        println!("{}", "-".repeat(72));
        let mut seen = std::collections::HashSet::new();
        for d in api.device_list() {
            if !seen.insert((d.vendor_id(), d.product_id())) {
                continue;
            }
            let ours = d.vendor_id() == want_vid && d.product_id() == want_pid;
            println!(
                "0x{:04x}   0x{:04x}    {:<28} {}{}",
                d.vendor_id(),
                d.product_id(),
                d.product_string().unwrap_or("?"),
                d.manufacturer_string().unwrap_or("?"),
                if ours { "   ← FireVibe 认这个" } else { "" }
            );
        }
        println!("\nFireVibe 会打开 VID 0x{want_vid:04x} / PID 0x{want_pid:04x}。");
        if (want_vid, want_pid) != (firevibe_core::device::VID, firevibe_core::device::PID) {
            println!("（来自配置里的 device_vid / device_pid 覆盖）");
        } else {
            println!("换一款遥控器：把上面查到的值填进配置的 settings.device_vid / device_pid。");
        }
        return Ok(());
    }

    if has("--inputs") {
        let cur = firevibe_core::audio::default_input();
        for d in firevibe_core::audio::input_devices() {
            let mark = if Some(d.id) == cur.as_ref().map(|c| c.id) { "*" } else { " " };
            println!("{mark} {:>5}  {}", d.id, d.name);
        }
        return Ok(());
    }

    if let Some(prefix) = val("--set-input") {
        let want = prefix.to_lowercase();
        let d = firevibe_core::audio::input_devices()
            .into_iter()
            .find(|d| d.name.to_lowercase().contains(&want))
            .ok_or_else(|| anyhow::anyhow!("没找到匹配 {prefix:?} 的输入设备"))?;
        firevibe_core::audio::set_default_input(d.id)?;
        std::thread::sleep(Duration::from_millis(300));
        println!("默认输入 → {}", firevibe_core::audio::default_input()
            .map(|x| x.name).unwrap_or_else(|| "?".into()));
        return Ok(());
    }

    if has("--sniff") {
        return sniff::run();
    }

    if has("--list-devices") {
        println!("输出设备:");
        println!("cpal 全量枚举：");
        for (n, o, i) in firevibe_core::voice::debug_devices() {
            println!("  {n:24} 出={o:22} 入={i}");
        }
        println!();
        for d in list_output_devices() {
            println!("  {d}");
        }
        return Ok(());
    }

    let mut cfg = Config::load();
    if has("--exclusive") {
        cfg.exclusive = true;
    }
    if let Some(d) = val("--device") {
        cfg.voice.device = d;
    }
    if let Some(m) = val("--mode") {
        cfg.voice.mode = if m == "always" { VoiceMode::Always } else { VoiceMode::Gate };
    }
    if let Some(g) = val("--gain").and_then(|g| g.parse::<f32>().ok()) {
        cfg.voice.gain = g;
    }
    if has("--no-voice") {
        cfg.voice.enabled = false;
    }

    println!("配置: {}", firevibe_core::config::config_path().display());
    let (rt, rx) = Runtime::new(cfg);
    println!("按键注入: 可用={}", rt.inj.available());
    if !rt.inj.available() {
        println!("  {}", rt.inj.why());
    }

    rt.start()?;

    if has("--descriptor") {
        std::thread::sleep(Duration::from_millis(300));
        let d = rt.descriptor.lock().clone();
        println!("报告描述符 {} 字节:", d.len());
        println!("{}", d.iter().map(|b| format!("{b:02x}")).collect::<String>());
        rt.stop();
        return Ok(());
    }

    if rt.cfg.read().voice.enabled {
        if let Err(e) = rt.start_voice() {
            println!("语音未启动: {e}");
        }
    }

    {
        let c = rt.cfg.read();
        println!("方案 {}（共 {} 套）  长按阈值 {}ms:",
                 c.profile().name, c.profiles.len(), c.long_press_ms());
        for a in &c.profile().actions {
            let usage = c
                .slot_key(a.slot)
                .map(|k| k.id())
                .unwrap_or_else(|| "未绑".into());
            let off = if a.disabled { "  [已禁用]" } else { "" };
            println!("  {:<12} {:<10} 短按 {:<26} 长按 {}{}",
                     a.slot.label(), usage,
                     a.short.describe(), a.long.describe(), off);
        }
    }
    println!("\n按遥控器上的键试试。Ctrl-C 退出。\n");

    let mut last_stat = std::time::Instant::now();
    loop {
        while let Ok(ev) = rx.try_recv() {
            match ev {
                Event::Key { key, down, result } => {
                    println!("  {} {:<24} {}", if down { "↓" } else { "↑" }, key.to_string(), result);
                }
                Event::Raw { report_id, data } => {
                    let hex: String = data.iter().map(|b| format!("{b:02X} ")).collect();
                    println!("  raw id=0x{report_id:02X} {hex}");
                }
                Event::Log(s) => println!("  · {s}"),
                Event::Connected { product, serial } => {
                    println!("  已连接: {product}  SN {serial}")
                }
                Event::Disconnected(e) => {
                    println!("  连接断开: {e}");
                    return Ok(());
                }
                Event::Learned(k) => println!("  学习到: {k}"),
            }
        }
        if last_stat.elapsed() >= Duration::from_secs(2) {
            last_stat = std::time::Instant::now();
            let s = &rt.status;
            let (passing, level, dropped) = match rt.voice_sink() {
                Some(v) => (v.passing(), v.level(), v.dropped()),
                None => (false, 0.0, 0),
            };
            print!(
                "\r  电量 {}%  麦克风 {}  送流 {}  已收 {} 帧  电平 {:.3}  丢 {}      ",
                s.battery.load(Ordering::Relaxed),
                s.mic_on.load(Ordering::Relaxed),
                passing,
                s.audio_frames.load(Ordering::Relaxed),
                level,
                dropped
            );
            use std::io::Write;
            let _ = std::io::stdout().flush();
        }
        std::thread::sleep(Duration::from_millis(30));
    }
}


#[link(name = "ApplicationServices", kind = "framework")]
unsafe extern "C" {
    fn CGEventSourceFlagsState(state: u32) -> u64;
    fn CGEventSourceKeyState(state: u32, keycode: u16) -> bool;
}



/// 修饰键对照：先让用户用真键盘按一次，再由我们合成一次，把 tap 看到的原始字段打出来。
fn run_modcmp(key: Option<String>) -> anyhow::Result<()> {
    use firevibe_core::tap;
    use std::sync::{Arc, Mutex};

    let key = key.unwrap_or_else(|| "rightoption".into());
    let log: Arc<Mutex<Vec<(String, tap::Ev)>>> = Arc::new(Mutex::new(Vec::new()));
    let phase = Arc::new(Mutex::new(String::from("键盘")));

    let l = log.clone();
    let ph = phase.clone();
    // listen_only = true：只看不拦
    let _t = tap::spawn(
        &[tap::EV_KEY_DOWN, tap::EV_KEY_UP, tap::EV_FLAGS_CHANGED],
        true,
        Box::new(move |ev| {
            l.lock().unwrap().push((ph.lock().unwrap().clone(), ev));
            false
        }),
        None,
    )?;

    // 事件字段之外还有一项事件里看不见的东西：系统的**全局修饰键状态**。
    // 靠轮询 NSEvent.modifierFlags / CGEventSourceFlagsState 判断「Option 按住了吗」
    // 的 app，只认这个，不认事件。
    let watch = |label: &str, secs: f32| {
        const ALT: u64 = 0x0008_0000;
        // 0 = CombinedSessionState（含本进程 posted 的事件）
        // 1 = HIDSystemState（纯硬件）
        let t0 = Instant::now();
        let (mut comb, mut hid, mut keyc, mut keyh, mut ns) = (false, false, false, false, false);
        while t0.elapsed().as_secs_f32() < secs {
            unsafe {
                if CGEventSourceFlagsState(0) & ALT != 0 { comb = true; }
                if CGEventSourceFlagsState(1) & ALT != 0 { hid = true; }
                if CGEventSourceKeyState(0, 61) { keyc = true; }
                if CGEventSourceKeyState(1, 61) { keyh = true; }
            }
            if firevibe_core::inject::ns_modifier_alt() { ns = true; }
            std::thread::sleep(Duration::from_millis(40));
        }
        let y = |b: bool| if b { "有 ✓" } else { "没有 ✗" };
        println!("   [{label}]");
        println!("     CombinedSessionState 的 option 位  {}", y(comb));
        println!("     HIDSystemState 的 option 位        {}", y(hid));
        println!("     KeyState(combined, 右option=61)    {}", y(keyc));
        println!("     KeyState(hid, 右option=61)         {}", y(keyh));
        println!("     NSEvent.modifierFlags 的 option    {}", y(ns));
    };

    if !std::env::args().any(|a| a == "--synth-only") {
        println!("① 现在用**真键盘**按住 {key} 三秒再松开…");
        watch("真键盘", 5.0);
    }

    *phase.lock().unwrap() = "合成".into();
    println!("② 换我们合成一次…");
    let inj = firevibe_core::inject::new_injector();
    inj.key_down(&key, &[])?;
    watch("合成", 1.2);
    inj.key_up(&key, &[])?;
    std::thread::sleep(Duration::from_millis(400));

    println!("\n来源   事件类型            键码    flags        nx_down  pid");
    println!("{}", "-".repeat(62));
    for (src, ev) in log.lock().unwrap().iter() {
        let kind = match ev.kind {
            x if x == tap::EV_KEY_DOWN => "keyDown",
            x if x == tap::EV_KEY_UP => "keyUp",
            x if x == tap::EV_FLAGS_CHANGED => "flagsChanged",
            other => return Err(anyhow::anyhow!("没见过的事件类型 {other}")),
        };
        println!(
            "{src:5}  {kind:18} 0x{:<4x}  0x{:<10x} {:<8} {}",
            ev.code, ev.flags, ev.nx_down, ev.pid
        );
    }
    println!("\n两组 flags 不一致就说明合成的和键盘不是一回事。");
    Ok(())
}


/// 盯着按键事件流：每个事件的键码、完整 flags、来源、按住时长都打出来。
/// 用真遥控器触发一次、再用真键盘按一次，两行并排就能直接比。
fn run_watch_mods() -> anyhow::Result<()> {
    use firevibe_core::tap;
    use std::collections::HashMap;
    use std::sync::{Arc, Mutex};

    fn key_name(code: i64) -> &'static str {
        match code {
            0x3a => "左⌥", 0x3d => "右⌥",
            0x37 => "左⌘", 0x36 => "右⌘",
            0x38 => "左⇧", 0x3c => "右⇧",
            0x3b => "左⌃", 0x3e => "右⌃",
            0x3f => "fn",  0x39 => "caps",
            _ => "",
        }
    }
    /// flags 里有哪些位，拆开说人话
    fn decode(f: u64) -> String {
        const BITS: &[(u64, &str)] = &[
            (0x0002_0000, "shift"), (0x0004_0000, "ctrl"), (0x0008_0000, "option"),
            (0x0010_0000, "cmd"), (0x0080_0000, "fn"), (0x0001_0000, "caps"),
            (0x0000_0100, "非合并"),
            (0x01, "左ctrl位"), (0x02, "左shift位"), (0x04, "右shift位"),
            (0x08, "左cmd位"), (0x10, "右cmd位"), (0x20, "左opt位"), (0x40, "右opt位"),
            (0x2000, "右ctrl位"), (0x2000_0000, "进程合成标记"),
        ];
        let mut v: Vec<&str> = BITS.iter().filter(|(b, _)| f & b != 0).map(|(_, n)| *n).collect();
        if v.is_empty() { v.push("无"); }
        v.join("+")
    }

    let down_at: Arc<Mutex<HashMap<i64, Instant>>> = Arc::new(Mutex::new(HashMap::new()));
    let t0 = Instant::now();
    println!("来源     ms      事件           键码        flags        位含义");
    println!("{}", "-".repeat(96));

    let d = down_at.clone();
    let _t = tap::spawn(
        &[tap::EV_KEY_DOWN, tap::EV_KEY_UP, tap::EV_FLAGS_CHANGED],
        true, // 只看不拦
        Box::new(move |ev| {
            let src = if ev.pid == 0 { "硬件" } else { "合成" };
            let kind = if ev.kind == tap::EV_FLAGS_CHANGED {
                "flagsChanged"
            } else if ev.kind == tap::EV_KEY_DOWN {
                "keyDown"
            } else {
                "keyUp"
            };
            // 修饰键的按下/松开靠「这个键对应的位还在不在」判断
            let mut extra = String::new();
            if ev.kind == tap::EV_FLAGS_CHANGED {
                let own = match ev.code {
                    0x3a | 0x3d => 0x0008_0000u64,
                    0x37 | 0x36 => 0x0010_0000,
                    0x38 | 0x3c => 0x0002_0000,
                    0x3b | 0x3e => 0x0004_0000,
                    0x3f => 0x0080_0000,
                    _ => 0,
                };
                let is_down = own != 0 && ev.flags & own != 0;
                let mut g = d.lock().unwrap();
                if is_down {
                    g.insert(ev.code, Instant::now());
                    extra = " 按下".into();
                } else if let Some(t) = g.remove(&ev.code) {
                    extra = format!(" 松开 按住了 {}ms", t.elapsed().as_millis());
                }
            }
            println!(
                "{src}   {:>6}  {kind:13}  0x{:02x} {:4}  0x{:<9x}  {}{}",
                t0.elapsed().as_millis(),
                ev.code,
                key_name(ev.code),
                ev.flags,
                decode(ev.flags),
                extra
            );
            false
        }),
        None,
    )?;

    println!("用**遥控器**触发一次，再用**真键盘**按一次同一个修饰键，对着比。Ctrl-C 退出。\n");
    loop {
        std::thread::sleep(Duration::from_millis(200));
    }
}
