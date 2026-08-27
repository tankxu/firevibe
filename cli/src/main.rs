//! 无界面版：在做 GUI 之前把引擎跑通，也用于排障。
mod adapt;
mod map;
mod probeall;
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


/// 让 firectl 成为自己的 TCC「责任进程」，从而能像 FireVibe.app 一样
/// 在系统设置里**独立持有**「输入监控」授权 —— 而不是继承启动它的终端那份。
///
/// 背景：从 shell 跑一个 CLI，TCC 把权限归责到父进程（终端）。所以哪怕 firectl
/// 自己签名再正确，用的还是终端那份授权；终端若是 ad-hoc 签名，授权还会静默失效。
/// 解法：进程一启动就用带 `disclaim` 的 posix_spawn 原地重执行自己（SETEXEC），
/// 重执行出来的这一份对 TCC 自负其责。之后在「输入监控」里勾一次 firectl 本体，
/// 从任何终端跑都认这份授权。
///
/// 首次生效需要用户在弹框里授权一次 firectl（跟当初授权 FireVibe 一样）。
#[cfg(target_os = "macos")]
fn become_self_responsible() {
    use std::ffi::CString;
    use std::os::unix::ffi::{OsStrExt, OsStringExt};
    // 已经重执行过 → 跳过，避免无限循环
    if std::env::var_os("FIRECTL_DISCLAIMED").is_some() {
        return;
    }
    extern "C" {
        // 私有 API（libSystem 里有，无公开头文件）：令 posix_spawn 出的进程
        // 对 TCC 自负其责，不再继承父进程的责任归属。
        fn responsibility_spawnattrs_setdisclaim(
            attr: *mut libc::posix_spawnattr_t,
            disclaim: libc::c_int,
        ) -> libc::c_int;
    }
    let Ok(exe) = std::env::current_exe() else { return };
    let Ok(exe_c) = CString::new(exe.as_os_str().as_bytes()) else { return };

    // argv 原样透传
    let args: Vec<CString> = std::env::args_os()
        .filter_map(|a| CString::new(a.as_bytes()).ok())
        .collect();
    let mut argv: Vec<*mut libc::c_char> = args.iter().map(|a| a.as_ptr() as *mut _).collect();
    argv.push(std::ptr::null_mut());

    // envp 透传 + 打上标记，防止重执行后再次进来
    let mut envs: Vec<CString> = std::env::vars_os()
        .filter_map(|(k, v)| {
            let mut kv = k.into_vec();
            kv.push(b'=');
            kv.extend_from_slice(v.as_bytes());
            CString::new(kv).ok()
        })
        .collect();
    if let Ok(m) = CString::new("FIRECTL_DISCLAIMED=1") {
        envs.push(m);
    }
    let mut envp: Vec<*mut libc::c_char> = envs.iter().map(|e| e.as_ptr() as *mut _).collect();
    envp.push(std::ptr::null_mut());

    unsafe {
        let mut attr: libc::posix_spawnattr_t = std::mem::zeroed();
        if libc::posix_spawnattr_init(&mut attr) != 0 {
            return;
        }
        responsibility_spawnattrs_setdisclaim(&mut attr, 1);
        // SETEXEC：像 exec 一样原地替换本进程镜像，但带上 disclaim 属性。
        // 成功则不返回；返回了就是失败，退回原流程（最坏 == 改动前的行为）。
        libc::posix_spawnattr_setflags(&mut attr, libc::POSIX_SPAWN_SETEXEC as libc::c_short);
        libc::posix_spawn(
            std::ptr::null_mut(),
            exe_c.as_ptr(),
            std::ptr::null(),
            &attr,
            argv.as_ptr(),
            envp.as_ptr(),
        );
        libc::posix_spawnattr_destroy(&mut attr);
    }
}


/// 定死哪个关麦命令真能停流。直接用 hidapi 开设备，不经 runtime（免得开局补发/
/// 自愈那些自动发命令干扰）。开麦→测基线帧率→发候选关麦→测之后帧率，逐个比。
fn mic_off_test() -> anyhow::Result<()> {
    use std::time::{Duration, Instant};
    let cfg = firevibe_core::config::Config::load();
    let (vid, pid) = cfg.device_ids();
    let api = hidapi::HidApi::new()?;
    #[cfg(target_os = "macos")]
    api.set_open_exclusive(false); // 非独占，跟 runtime 一致；默认独占会 privilege violation
    let dev = api.open(vid, pid).map_err(|e| anyhow::anyhow!("HID_NOT_PERMITTED: {e}"))?;
    dev.set_blocking_mode(false).ok();
    println!("已打开 0x{vid:04x}/0x{pid:04x}\n");

    // 数 secs 秒内收到多少条 0xF0 音频帧
    let count = |dev: &hidapi::HidDevice, secs: f32| -> u32 {
        let mut buf = [0u8; 128];
        let mut n = 0u32;
        let end = Instant::now() + Duration::from_secs_f32(secs);
        while Instant::now() < end {
            if let Ok(len) = dev.read_timeout(&mut buf, 50) {
                if len > 0 && buf[0] == 0xF0 {
                    n += 1;
                }
            }
        }
        n
    };
    let arm = |dev: &hidapi::HidDevice| {
        let _ = dev.write(&[0xF2, 0x01, 0x01]); // 已知有效的开麦
        std::thread::sleep(Duration::from_millis(800));
    };

    // 候选关麦命令
    let candidates: &[(&str, &[u8])] = &[
        ("[F2 01 00]  3字节（旧关麦，实测无效）", &[0xF2, 0x01, 0x00]),
        ("[F2 00]     2字节（现用关麦）", &[0xF2, 0x00]),
    ];

    println!("先开麦，测基线帧率（应 ~50/秒）…");
    arm(&dev);
    let base = count(&dev, 3.0);
    println!("  基线 {} 帧 / 3秒 = {:.0}/秒\n", base, base as f32 / 3.0);
    if base < 30 {
        println!("⚠ 基线太低，麦克风没在吐流（可能没按住/没热）。先确认 --mic 能出流再来。");
        return Ok(());
    }

    for (label, cmd) in candidates {
        arm(&dev); // 每轮先确保在开麦态
        let _ = count(&dev, 1.0); // 稳定一下
        let w = dev.write(cmd);
        std::thread::sleep(Duration::from_millis(600)); // 给停流留反应时间
        let after = count(&dev, 3.0);
        let rate = after as f32 / 3.0;
        let verdict = if rate < 5.0 { "✓ 停了" } else if rate < 25.0 { "~ 半停" } else { "✗ 没停" };
        println!(
            "{label:32} 写={:<6} 之后 {rate:4.0}/秒  {verdict}",
            w.map(|n| format!("{n}B")).unwrap_or_else(|_| "失败".into())
        );
    }
    // 收尾：用测出来有效的那个关，兜底两个都发
    let _ = dev.write(&[0xF2, 0x00]);
    let _ = dev.write(&[0xF2, 0x01, 0x00]);
    println!("\n（已尝试关麦收尾）");
    Ok(())
}


/// 按键边沿追踪：每个键的**按下 / 松开**都带时间戳打印，不执行任何动作。
/// 专门诊断「短按先于长按触发」——按住一个键 2 秒，看它中途会不会
/// 冒出一次「松开→按下」（BLE 瞬断），那就是短按被提前触发的根因。
fn key_trace() -> anyhow::Result<()> {
    use firevibe_core::runtime::Event;
    use std::sync::atomic::Ordering;
    use std::time::Instant;
    let cfg = firevibe_core::config::Config::load();
    let (rt, rx) = firevibe_core::runtime::Runtime::new(cfg);
    rt.start()?;
    rt.set_learn(true); // 只观测，不执行动作
    rt.trace_keys.store(true, Ordering::Relaxed);

    println!("\n按键边沿追踪 —— 每个按下/松开都会打出来（带时间戳，单位毫秒）。");
    println!("请**按住那个配了短按+长按的键约 2 秒，再松开**。可重复几次。");
    println!("正常长按应只有一条「按下」…（2秒）…一条「松开」；");
    println!("要是中途冒出「松开」紧接「按下」，就是 BLE 瞬断 —— 短按被提前触发的元凶。");
    println!("30 秒后自动结束，Ctrl-C 也行。");
    println!("{}", "─".repeat(60));

    let t0 = Instant::now();
    let mut last: Option<(bool, f64)> = None;
    while t0.elapsed().as_secs() < 30 {
        while let Ok(ev) = rx.try_recv() {
            if let Event::KeyEdge { key, down } = ev {
                let ms = t0.elapsed().as_secs_f64() * 1000.0;
                // 标出与上一条的间隔，瞬断一眼可见
                let gap = last.map(|(_, t)| format!("(+{:.0}ms)", ms - t)).unwrap_or_default();
                let flag = match last {
                    Some((true, t)) if down && (ms - t) < 300.0 => "  ⚠ 按下→按下",
                    Some((false, t)) if down && (ms - t) < 120.0 => "  ⚠⚠ 松开紧接按下 = 瞬断！",
                    _ => "",
                };
                println!(
                    "[{ms:8.0}ms] {:<6} {key}   {gap}{flag}",
                    if down { "按下" } else { "松开" }
                );
                last = Some((down, ms));
            }
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    rt.trace_keys.store(false, Ordering::Relaxed);
    rt.stop();
    println!("{}", "─".repeat(60));
    println!("把上面整段发回来。");
    Ok(())
}


/// 开麦载荷探测 —— 这台遥控器描述符和原厂一样、却不吃原厂开麦命令时用。
///
/// ⚠️ **只在已知的命令通道 `0xF2` 上试**，载荷都是原厂那套的变体
/// （`01 01` 系列 + 单字节 `01`）。**不碰 `0xF3`**（10 字节 OUTPUT、语义未知），
/// 也**不做 opcode 盲扫** —— 对不明设备扫厂商命令可能干出不可逆的事。
/// 逐个 collection 试开麦。
///
/// macOS 把一份含多个 top-level Application Collection 的报告描述符拆成**多个
/// IOHIDDevice**，VID/PID 一模一样。音频输入报告 0xF0 和开麦命令 0xF2 都只在
/// Vendor FF00 那个 collection 上。`hidapi::open(vid, pid)` 取枚举里第一个 ——
/// 挑错了 SetReport 照样返回成功（macOS 不校验 report id 属不属于这个 collection），
/// 但 0xF0 永远收不到。0x0421 能用、0x0425 不能用，很可能就差在这。
///
/// 只发已验证的 MIC_ON / MIC_OFF，不做厂商 opcode 扫描。
fn collection_test() -> anyhow::Result<()> {
    use firevibe_core::device::{MIC_OFF, MIC_ON};
    use std::time::{Duration, Instant};

    let cfg = firevibe_core::config::Config::load();
    let (vid, pid) = cfg.device_ids();
    let api = hidapi::HidApi::new()?;
    #[cfg(target_os = "macos")]
    api.set_open_exclusive(false);

    let hits: Vec<_> = api
        .device_list()
        .filter(|d| d.vendor_id() == vid && d.product_id() == pid)
        .collect();
    if hits.is_empty() {
        println!("没找到 0x{vid:04x}/0x{pid:04x} —— 遥控器没连上？");
        return Ok(());
    }
    println!("0x{vid:04x}/0x{pid:04x} 共 {} 个 collection：\n", hits.len());

    let mut best: Option<(u16, u16, u32)> = None;
    for (i, d) in hits.iter().enumerate() {
        let (up, u) = (d.usage_page(), d.usage());
        print!("[{}/{}] usage_page 0x{up:04x} usage 0x{u:02x} … ", i + 1, hits.len());
        use std::io::Write;
        std::io::stdout().flush().ok();

        let dev = match api.open_path(d.path()) {
            Ok(x) => x,
            Err(e) => {
                println!("打不开：{e}");
                continue;
            }
        };
        dev.set_blocking_mode(false).ok();

        if let Err(e) = dev.write(&MIC_ON) {
            println!("写 MIC_ON 失败：{e}");
            continue;
        }
        // 数 3 秒 0xF0，中途每秒补一次 keepalive（开麦命令会过期）
        let mut buf = [0u8; 128];
        let (mut frames, mut others) = (0u32, Vec::<u8>::new());
        let start = Instant::now();
        let mut next_ka = start + Duration::from_secs(1);
        while start.elapsed() < Duration::from_secs(3) {
            if Instant::now() >= next_ka {
                let _ = dev.write(&MIC_ON);
                next_ka += Duration::from_secs(1);
            }
            if let Ok(len) = dev.read_timeout(&mut buf, 50) {
                if len > 0 {
                    if buf[0] == 0xF0 {
                        frames += 1;
                    } else if !others.contains(&buf[0]) {
                        others.push(buf[0]);
                    }
                }
            }
        }
        let _ = dev.write(&MIC_OFF);

        let extra = if others.is_empty() {
            String::new()
        } else {
            format!("，另见 report {:02x?}", others)
        };
        println!("{frames} 帧 0xF0{extra}");
        if frames > 0 && best.map_or(true, |(_, _, n)| frames > n) {
            best = Some((up, u, frames));
        }
    }

    println!();
    match best {
        Some((up, u, n)) => {
            println!("★ 出流的是 usage_page 0x{up:04x} / usage 0x{u:02x}（{n} 帧）");
            println!("  → FireVibe 打开设备时要认准这个 collection，不能用 open(vid,pid)。");
        }
        None => println!("所有 collection 都是 0 帧 —— 不是选错 collection 的问题。"),
    }
    Ok(())
}

/// 开着麦克风蹲 20 秒，把**所有**收到的 report id 按数量列出来。
///
/// 用来分两种情况：
///   - 除了 0xF0 还能收到别的 vendor 输入（0xF1 / 0xEF / 电池 0x03）→ 订阅通的，
///     问题在遥控器不肯出音频；
///   - 一条都收不到 → macOS 没给这个 collection 的输入报告开 CCCD 订阅。
/// 同时也测「是不是必须真按住麦克风键」—— 官方那台是热麦克风，按不按都出流。
fn mic_listen() -> anyhow::Result<()> {
    use firevibe_core::device::{MIC_OFF, MIC_ON};
    use std::collections::BTreeMap;
    use std::time::{Duration, Instant};

    let cfg = firevibe_core::config::Config::load();
    let (vid, pid) = cfg.device_ids();
    let api = hidapi::HidApi::new()?;
    #[cfg(target_os = "macos")]
    api.set_open_exclusive(false);

    // 遥控器空闲几十秒就休眠掉线，所以在这儿等它上线，别让用户去掐时间。
    // 认准 vendor collection（描述符里 06 ff 00 → usage page 0x00ff，usage 0x00）
    let mut api = api;
    let mut waited = 0u32;
    let path = loop {
        let pick = api
            .device_list()
            .find(|d| d.vendor_id() == vid && d.product_id() == pid && d.usage_page() == 0x00ff)
            .or_else(|| {
                api.device_list()
                    .find(|d| d.vendor_id() == vid && d.product_id() == pid)
            });
        if let Some(d) = pick {
            println!(
                "打开 usage_page 0x{:04x} usage 0x{:02x}",
                d.usage_page(),
                d.usage()
            );
            break d.path().to_owned();
        }
        if waited == 0 {
            println!("等遥控器上线 —— 按一下它上面任意一个键唤醒（最多等 90 秒）…");
        }
        if waited >= 90 {
            anyhow::bail!("等了 90 秒没等到 0x{vid:04x}/0x{pid:04x}");
        }
        std::thread::sleep(std::time::Duration::from_secs(1));
        waited += 1;
        api.refresh_devices()?;
    };
    let info_path = path;
    let dev = api.open_path(&info_path)?;
    dev.set_blocking_mode(false).ok();

    // --no-cmd：完全不发命令，只靠物理按键 —— 用来 A/B 出 MIC_ON 到底需不需要
    let no_cmd = std::env::args().any(|a| a == "--no-cmd");
    if no_cmd {
        println!("【对照组】不发任何命令，只靠按住麦克风键。\n");
    } else {
        dev.write(&MIC_ON)
            .map_err(|e| anyhow::anyhow!("写 MIC_ON 失败（多半是没有输入监控权限）：{e}"))?;
        println!("MIC_ON 已发。\n");
    }
    // 窗口时长可调 —— 和用户对时间点太难，默认放长一点，什么时候按都行
    let secs: u64 = std::env::args()
        .find_map(|a| a.strip_prefix("--secs=").and_then(|v| v.parse().ok()))
        .unwrap_or(20);
    println!("接下来 {secs} 秒内**随便什么时候**按住麦克风键说话都行，按住 3 秒松开，重复几次。\n");

    // 顺带解码：光数帧不够，要知道声音本身响不响 —— 电平只有一格时得分清
    // 是「帧太少」还是「帧有但几乎静音」。
    let mut dec = opus::Decoder::new(16_000, opus::Channels::Mono)
        .map_err(|e| anyhow::anyhow!("opus 解码器建不起来：{e}"))?;
    let mut pcm = vec![0i16; 320 * 6];
    let (mut peak, mut sumsq, mut nsamp, mut decode_err) = (0i32, 0f64, 0u64, 0u32);

    let mut buf = [0u8; 128];
    let mut tally: BTreeMap<u8, u32> = BTreeMap::new();
    let mut first_seen: BTreeMap<u8, f32> = BTreeMap::new();
    let start = Instant::now();
    let mut next_ka = start + Duration::from_secs(1);
    let mut next_tick = start + Duration::from_secs(1);
    while start.elapsed() < Duration::from_secs(secs) {
        let now = Instant::now();
        if now >= next_ka {
            if !no_cmd {
                let _ = dev.write(&MIC_ON); // keepalive：开麦命令会过期
            }
            next_ka += Duration::from_secs(1);
        }
        if now >= next_tick {
            let t = start.elapsed().as_secs();
            let total: u32 = tally.values().sum();
            print!("\r  {t:>2}s  已收 {total} 条  ");
            use std::io::Write;
            std::io::stdout().flush().ok();
            next_tick += Duration::from_secs(1);
        }
        if let Ok(len) = dev.read_timeout(&mut buf, 20) {
            if len > 0 {
                let id = buf[0];
                // 头一帧音频把长度和 Opus TOC 打出来 —— 确认和官方遥控器同格式
                // （官方：81 字节 = 1 报告 ID + 80 字节包，TOC 恒为 0xB8）
                if id == 0xF0 && !tally.contains_key(&0xF0) {
                    println!("\n  首帧 0xF0：len={len}  TOC=0x{:02X}  前16字节={:02x?}",
                             buf[1], &buf[..16.min(len)]);
                }
                if id == 0xF0 {
                    match dec.decode(&buf[1..len], &mut pcm, false) {
                        Ok(got) => {
                            for &v in &pcm[..got] {
                                let a = (v as i32).abs();
                                if a > peak {
                                    peak = a;
                                }
                                sumsq += (v as f64) * (v as f64);
                            }
                            nsamp += got as u64;
                        }
                        Err(_) => decode_err += 1,
                    }
                }
                *tally.entry(id).or_insert(0) += 1;
                first_seen
                    .entry(id)
                    .or_insert_with(|| start.elapsed().as_secs_f32());
            }
        }
    }
    if !no_cmd {
        let _ = dev.write(&MIC_OFF);
    }
    println!("\n\n──────── 收到的 report ────────");
    if tally.is_empty() {
        println!("  一条都没有。");
        println!("  → 这个 collection 的输入报告根本没进来（macOS 没开 CCCD 订阅，");
        println!("    或者遥控器在这条通道上什么都不发）。");
    } else {
        for (id, n) in &tally {
            let t = first_seen[id];
            let what = match id {
                0xF0 => "音频（Opus）",
                0xF1 => "vendor，未知",
                0xEF => "App 快捷键",
                0x03 => "电池",
                0x01 => "键盘",
                0x02 => "Consumer（麦克风键在这）",
                _ => "?",
            };
            println!("  0x{id:02X}  {n:>5} 条   首次 +{t:.1}s   {what}");
        }
        if tally.contains_key(&0xF0) {
            let n = tally[&0xF0];
            println!("\n★ 收到音频：{n} 帧 ≈ {:.2}s", n as f32 * 0.02);
            if nsamp > 0 {
                let rms = (sumsq / nsamp as f64).sqrt();
                // 16 位满量程 32767；-30 dBFS 以下基本等于没说话
                let db = 20.0 * (rms.max(1.0) / 32767.0).log10();
                let pdb = 20.0 * ((peak.max(1) as f64) / 32767.0).log10();
                println!(
                    "  解出 {nsamp} 采样  RMS {rms:.0} ({db:.1} dBFS)  峰值 {peak} ({pdb:.1} dBFS)"
                );
                if pdb < -40.0 {
                    println!("  ⚠ 峰值 <-40dBFS —— 遥控器送来的就是近乎静音，不是 FireVibe 的锅");
                } else if pdb > -12.0 {
                    println!("  音量正常，问题在下游（虚拟声卡 / 增益 / 采样率）");
                } else {
                    println!("  音量偏小但有内容");
                }
            }
            if decode_err > 0 {
                println!("  ⚠ {decode_err} 帧解码失败 —— 格式可能和官方那台不一样");
            }
        } else {
            println!("\n没有 0xF0，但别的输入报告进得来 → 订阅是通的，是遥控器不肯出音频。");
        }
    }
    Ok(())
}

/// 一条命令跑完麦克风适配检查，全程在终端里给提示和倒计时。
///
/// 分三段，用来把两种开麦模型和「压根不出流」区分开：
///   ① 静默基线    —— 什么都不发、不碰遥控器
///   ② 热麦克风     —— 发 MIC_ON，仍然不碰遥控器（0x0421 这时就该出流）
///   ③ 按住         —— 按住物理麦克风键说话（0x0425 只有这时才出流）
/// 顺带解码算音量，分清「没帧」和「有帧但近乎静音」。
fn mic_check() -> anyhow::Result<()> {
    use firevibe_core::device::{MIC_OFF, MIC_ON};
    use std::io::Write;
    use std::time::{Duration, Instant};

    println!("\n  FireVibe 麦克风适配检查");
    println!("  ─────────────────────────────────────────\n");

    // app 也在读同一台设备时，报告可能只送到其中一个进程
    let app_running = std::process::Command::new("pgrep")
        .args(["-f", "FireVibe.app/Contents/MacOS/firevibe"])
        .output()
        .map(|o| !o.stdout.is_empty())
        .unwrap_or(false);
    if app_running {
        println!("  ⚠ FireVibe.app 正在运行。两个进程读同一台 HID 设备时，报告可能只");
        println!("    送到其中一个。这一步测出 0 帧的话，先退出 app 再跑一次。\n");
    }

    let cfg = firevibe_core::config::Config::load();
    let (vid, pid) = cfg.device_ids();
    let mut api = hidapi::HidApi::new()?;
    #[cfg(target_os = "macos")]
    api.set_open_exclusive(false);

    // ── 等设备 ──
    print!("  ① 等遥控器上线 —— 按一下它上面任意一个键唤醒 ");
    std::io::stdout().flush().ok();
    let mut waited = 0u32;
    let (path, label) = loop {
        let pick = api
            .device_list()
            .find(|d| d.vendor_id() == vid && d.product_id() == pid && d.usage_page() == 0x00ff)
            .or_else(|| {
                api.device_list()
                    .find(|d| d.vendor_id() == vid && d.product_id() == pid)
            });
        if let Some(d) = pick {
            break (
                d.path().to_owned(),
                format!(
                    "{} 0x{vid:04x}/0x{pid:04x} usage_page 0x{:04x}",
                    d.product_string().unwrap_or("?"),
                    d.usage_page()
                ),
            );
        }
        if waited >= 120 {
            println!();
            anyhow::bail!("等了 120 秒没等到 0x{vid:04x}/0x{pid:04x}");
        }
        print!(".");
        std::io::stdout().flush().ok();
        std::thread::sleep(Duration::from_secs(1));
        waited += 1;
        api.refresh_devices()?;
    };
    println!("\n     ✓ {label}\n");

    let dev = api.open_path(&path)?;
    dev.set_blocking_mode(false).ok();

    let mut dec = opus::Decoder::new(16_000, opus::Channels::Mono)
        .map_err(|e| anyhow::anyhow!("opus 解码器建不起来：{e}"))?;
    let mut pcm = vec![0i16; 320 * 6];

    // 跑一段：secs 秒，keepalive 决定要不要每秒重发 MIC_ON。
    // 返回 (0xF0 帧数, 峰值, RMS, 解码失败数, 见到的其它 report id)
    let mut run = |dev: &hidapi::HidDevice,
                   dec: &mut opus::Decoder,
                   secs: u64,
                   keepalive: bool,
                   hint: &str|
     -> (u32, i32, f64, u32, Vec<u8>) {
        let (mut frames, mut peak, mut sumsq, mut nsamp, mut errs) = (0u32, 0i32, 0f64, 0u64, 0u32);
        let mut others: Vec<u8> = Vec::new();
        let mut buf = [0u8; 128];
        let start = Instant::now();
        let mut next_ka = start + Duration::from_secs(1);
        let mut next_tick = start;
        while start.elapsed() < Duration::from_secs(secs) {
            let now = Instant::now();
            if keepalive && now >= next_ka {
                let _ = dev.write(&MIC_ON);
                next_ka += Duration::from_secs(1);
            }
            if now >= next_tick {
                let left = secs - start.elapsed().as_secs().min(secs);
                print!("\r     {hint}  还剩 {left:>2}s   已收 {frames} 帧   ");
                std::io::stdout().flush().ok();
                next_tick += Duration::from_secs(1);
            }
            if let Ok(len) = dev.read_timeout(&mut buf, 20) {
                if len > 0 {
                    if buf[0] == 0xF0 {
                        frames += 1;
                        match dec.decode(&buf[1..len], &mut pcm, false) {
                            Ok(got) => {
                                for &v in &pcm[..got] {
                                    let a = (v as i32).abs();
                                    if a > peak {
                                        peak = a;
                                    }
                                    sumsq += (v as f64) * (v as f64);
                                }
                                nsamp += got as u64;
                            }
                            Err(_) => errs += 1,
                        }
                    } else if !others.contains(&buf[0]) {
                        others.push(buf[0]);
                    }
                }
            }
        }
        let rms = if nsamp > 0 {
            (sumsq / nsamp as f64).sqrt()
        } else {
            0.0
        };
        print!("\r                                                                   \r");
        (frames, peak, rms, errs, others)
    };

    // ── ② 静默基线 ──
    println!("  ② 静默基线（5 秒）—— 请**不要碰遥控器**");
    let (a_n, ..) = run(&dev, &mut dec, 5, false, "     静默中");
    println!("     → {a_n} 帧\n");

    // ── ③ 热麦克风 ──
    println!("  ③ 热麦克风测试（6 秒）—— 发 MIC_ON，仍然**不要碰遥控器**");
    let _ = dev.write(&MIC_ON);
    let (b_n, b_pk, ..) = run(&dev, &mut dec, 6, true, "     已发 MIC_ON");
    let _ = dev.write(&MIC_OFF);
    println!("     → {b_n} 帧\n");

    // ── ④ 按住 ──
    println!("  ④ 按住测试（12 秒）—— 请**按住麦克风键，正常音量说话**");
    println!("     按住 3 秒松开，重复三四次");
    let (c_n, c_pk, c_rms, c_err, c_other) = run(&dev, &mut dec, 12, false, "     请按住说话");
    println!("     → {c_n} 帧");
    if c_n > 0 {
        let pdb = 20.0 * ((c_pk.max(1) as f64) / 32767.0).log10();
        let rdb = 20.0 * (c_rms.max(1.0) / 32767.0).log10();
        println!("       ≈ {:.2}s 音频   RMS {rdb:.1} dBFS   峰值 {pdb:.1} dBFS", c_n as f32 * 0.02);
        if c_err > 0 {
            println!("       ⚠ {c_err} 帧解码失败 —— 音频格式可能和官方那台不一样");
        }
    }
    if !c_other.is_empty() {
        println!("       另见 report {c_other:02X?}");
    }

    // ── 结论 ──
    println!("\n  ─────────────────────────────────────────");
    if b_n > 5 && c_n <= b_n / 4 {
        println!("  结论：**热麦克风** —— 发 MIC_ON 就一直出流，跟按键无关。");
        println!("  配置：麦克风键用「点一下」模式即可。");
    } else if c_n > 5 && b_n <= 5 {
        println!("  结论：**按住才出流（PTT）** —— MIC_ON 无效，必须按住物理麦克风键。");
        println!("  配置：麦克风键必须放在**短按槽 + 按住模式**；");
        println!("        放长按槽会漏掉按下那一下（弹 Spotlight），开头音频也丢。");
    } else if b_n > 5 && c_n > 5 {
        println!("  结论：两种都出流。按热麦克风用即可。");
    } else {
        println!("  结论：**一帧都没有。** 依次排查：");
        println!("    · 刚才第 ④ 步真的按住麦克风键了吗（不是点一下）");
        if app_running {
            println!("    · 退出 FireVibe.app 再跑一次（两个进程抢同一台设备）");
        }
        println!("    · 遥控器是不是「蓝牙连着但 HID 管道死了」—— 重新配对一次");
        println!("    · 静默基线也是 0 帧属正常；但连按键都收不到就是链路问题");
    }
    let _ = dev.write(&MIC_OFF);
    let _ = b_pk;
    println!();
    Ok(())
}

/// 从内置码库挑一条红外码发出去。用来在没绑按键的情况下直接验发射通路。
///
/// 用法：`firectl --ir-blast "daikin arc480a41" COOL`
/// 只给设备名 / 不给按键名 → 列出候选，不发射。
fn ir_blast_cmd(args: &[String]) -> anyhow::Result<()> {
    use firevibe_core::irdb;
    let pos: Vec<&String> = args.iter().filter(|a| !a.starts_with("--")).collect();
    let Some(q) = pos.first() else {
        println!("用法：firectl --ir-blast <设备搜索词> [按键名]");
        return Ok(());
    };
    // IRBLAST_RAW：直接发一串自定义时长（逗号/空格分隔），用来做协议实验，
    // 不经过码库。比如扫「多少个脉冲开始出问题」。
    if let Ok(raw) = std::env::var("IRBLAST_RAW") {
        let code = firevibe_core::ir::IrCode::parse(&raw).map_err(|e| anyhow::anyhow!(e))?;
        return ir_send(&code);
    }
    let hits = irdb::search(q, 20);
    if hits.is_empty() {
        println!("码库里搜不到「{q}」");
        return Ok(());
    }
    if hits.len() > 1 && pos.len() < 2 {
        println!("匹配到 {} 个，再写细一点：", hits.len());
        for h in &hits {
            println!("  {} {}  （{} · {} 键）", h.brand, h.model, h.category, h.buttons);
        }
        return Ok(());
    }
    let h = &hits[0];
    let btns = irdb::buttons_of(h.idx);
    println!("设备：{} {}  （{}）", h.brand, h.model, h.category);
    let Some(want) = pos.get(1) else {
        println!("按键：{}", btns.iter().map(|(n, _)| n.as_str()).collect::<Vec<_>>().join("  "));
        println!("\n再跑一次并带上按键名就会发射。");
        return Ok(());
    };
    let Some(bi) = btns.iter().position(|(n, _)| n.eq_ignore_ascii_case(want)) else {
        println!("没有叫「{want}」的按键。有这些：");
        println!("  {}", btns.iter().map(|(n, _)| n.as_str()).collect::<Vec<_>>().join("  "));
        return Ok(());
    };
    let code = irdb::code_of(h.idx, bi).ok_or_else(|| anyhow::anyhow!("这条码取不出来"))?;
    println!("按键：{}", btns[bi].0);
    return ir_send(&code);
}

/// 把一条码编译成表并交给 helper 发出去
fn ir_send(code: &firevibe_core::ir::IrCode) -> anyhow::Result<()> {
    // scanId 是「这一行挂在哪个物理键上」。一次性发射理论上无所谓，但这是目前
    // 唯一还在猜的参数，留个口子好扫。
    let scan_id: u8 = std::env::var("IRBLAST_SCANID")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(0);
    let table = code.compile_blast(scan_id).map_err(|e| anyhow::anyhow!(e))?;
    println!("{}", code.summary());
    println!("表 {} 字节，分 {} 片写，scanId={scan_id}", table.len(), (table.len() + 199) / 200);

    // 蓝牙那半交给独立小进程 —— 和 app 里走的是同一个 helper
    // FIREVIBE_IRBLAST 可以指到新编的 helper，改 helper 不用整包重签
    let exe = std::env::var("FIREVIBE_IRBLAST")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|_| "/Applications/FireVibe.app/Contents/MacOS/irblast".into());
    let exe = exe.as_path();
    if !exe.is_file() {
        anyhow::bail!("找不到 {} —— 先跑一次 ./package.sh 并装到 /Applications", exe.display());
    }
    // 蓝牙外设名。命令行里没有 runtime 在跑，battery::target_name() 是空的，
    // 所以直接从系统里找一台带 KeyMap 服务的遥控器 —— 让 helper 自己按名字挑。
    let dev = std::env::var("FIREVIBE_BT_NAME").unwrap_or_else(|_| "BLE".into());
    let hex: String = table.iter().map(|b| format!("{b:02x}")).collect();
    println!("发给「{dev}」…\n");
    // 调试期：IRBLAST_ARGS 里的开关原样透传给 helper（--verify N / --sha / --uuid-rand）
    let mut cmd = std::process::Command::new(exe);
    cmd.arg(&dev).arg(&hex);
    if let Ok(extra) = std::env::var("IRBLAST_ARGS") {
        for a in extra.split_whitespace() {
            cmd.arg(a);
        }
    }
    let st = cmd.status()?;
    println!();
    if st.success() {
        println!("✓ 设备回执正常 —— 空调有反应吗？");
    } else {
        println!("✗ 退出码 {:?}（上面几行是它卡在哪一步）", st.code());
    }
    Ok(())
}

fn mic_probe() -> anyhow::Result<()> {
    use std::time::{Duration, Instant};
    let cfg = firevibe_core::config::Config::load();
    let (vid, pid) = cfg.device_ids();
    let api = hidapi::HidApi::new()?;
    #[cfg(target_os = "macos")]
    api.set_open_exclusive(false);
    let dev = api.open(vid, pid).map_err(|e| anyhow::anyhow!("HID_NOT_PERMITTED: {e}"))?;
    dev.set_blocking_mode(false).ok();
    println!("已打开 0x{vid:04x}/0x{pid:04x}");
    println!("在已知命令通道 0xF2 上逐个试开麦载荷，每个发完看 3 秒 0xF0。\n");

    // 数 secs 秒内的 0xF0 帧；顺带记下见到的其它 report id
    let probe = |dev: &hidapi::HidDevice, secs: f32| -> (u32, Vec<u8>) {
        let mut buf = [0u8; 128];
        let (mut n, mut ids) = (0u32, Vec::<u8>::new());
        let end = Instant::now() + Duration::from_secs_f32(secs);
        while Instant::now() < end {
            if let Ok(len) = dev.read_timeout(&mut buf, 50) {
                if len > 0 {
                    if buf[0] == 0xF0 {
                        n += 1;
                    } else if !ids.contains(&buf[0]) {
                        ids.push(buf[0]);
                    }
                }
            }
        }
        (n, ids)
    };

    // 原厂那套的变体。第一个字节是 report id 0xF2，后面是载荷。
    let cands: &[(&str, &[u8])] = &[
        ("[F2 01 01]        原厂开麦", &[0xF2, 0x01, 0x01]),
        ("[F2 01 01 +0x8]   补满 10 字节载荷", &[0xF2, 0x01, 0x01, 0, 0, 0, 0, 0, 0, 0, 0]),
        ("[F2 01]           单字节 01", &[0xF2, 0x01]),
        ("[F2 02]           单字节 02", &[0xF2, 0x02]),
        ("[F2 01 02]        第二字节 02", &[0xF2, 0x01, 0x02]),
        ("[F2 03]           单字节 03", &[0xF2, 0x03]),
    ];

    let mut best: Option<(&str, u32)> = None;
    for (label, cmd) in cands {
        // 每轮先关麦，排掉上一轮残留
        let _ = dev.write(&[0xF2, 0x00]);
        std::thread::sleep(Duration::from_millis(300));
        let _ = probe(&dev, 0.4); // 清掉缓冲

        let w = dev.write(cmd);
        std::thread::sleep(Duration::from_millis(400));
        let (n, ids) = probe(&dev, 3.0);
        let extra = if ids.is_empty() {
            String::new()
        } else {
            format!(
                "  其它报文: {}",
                ids.iter().map(|i| format!("0x{i:02X}")).collect::<Vec<_>>().join(" ")
            )
        };
        println!(
            "{label:34} 写={:<8} 0xF0 {n:>4} 帧{extra}",
            w.map(|b| format!("{b}B")).unwrap_or_else(|_| "失败".into())
        );
        if n > 0 && best.map(|(_, bn)| n > bn).unwrap_or(true) {
            best = Some((label, n));
        }
    }

    // 收尾：关麦，别把它留在开着的状态
    let _ = dev.write(&[0xF2, 0x00]);
    std::thread::sleep(Duration::from_millis(300));

    println!();
    match best {
        Some((label, n)) => {
            println!("✓ 有效开麦载荷：{label}（{n} 帧）");
            println!("  把 core/src/device.rs 的 MIC_ON 改成它即可。");
        }
        None => {
            println!("✗ 试过的载荷都没起流。");
            println!("  这台可能：① 语音走别的 report（看上面「其它报文」有没有可疑 id）");
            println!("            ② 需要先握手/配对到电视才解锁语音");
            println!("            ③ 固件压根没实现（描述符是照抄原厂的，声明不算数）");
        }
    }
    Ok(())
}

fn main() {
    if let Err(e) = run_cli() {
        let msg = format!("{e:#}");
        eprintln!("\n错误: {msg}");
        if msg.contains("HID_NOT_PERMITTED")
            || msg.contains("not permitted")
            || msg.contains("0xE00002E2")
        {
            perm_help();
        } else if msg.contains("HID_NOT_FOUND") {
            eprintln!("\n找不到遥控器。先确认：");
            eprintln!("  · 遥控器已在 系统设置 › 蓝牙 里连上（不是配在电视上）");
            eprintln!("  · 设备标识对：先跑 `firectl --probe-all` 选一次设备");
        }
        std::process::exit(1);
    }
}

/// 权限指引 —— 遇到 `not permitted (0xE00002E2)` 时打印。
/// 这是踩了一整轮才理清的，务必讲准：
fn perm_help() {
    let exe = std::env::current_exe()
        .map(|p| p.display().to_string())
        .unwrap_or_else(|_| "firectl".into());
    eprintln!("\n──────── 这是权限问题，不是设备/代码问题 ────────");
    eprintln!("读写遥控器（HID）需要「输入监控」权限。firectl 已经把自己声明成独立的");
    eprintln!("授权主体（disclaim），所以要授权的是 **firectl 本体**，不是启动它的终端 ——");
    eprintln!("加一次，之后从任何终端跑都认（包括 Fleet 这种签名不稳的）。");
    eprintln!();
    eprintln!("一次性设置：");
    eprintln!("  1. 打开 系统设置 › 隐私与安全性 › 输入监控");
    eprintln!("  2. 点左下角「+」，定位到这个文件并添加：");
    eprintln!("     {exe}");
    eprintln!("     （Finder 里按 ⌘⇧G 粘贴上面的路径即可跳过去）");
    eprintln!("  3. 确保它的开关是打开的，然后重新运行本命令。");
    eprintln!();
    eprintln!("⚠️ 要的是「输入监控」(Input Monitoring)，不是「辅助功能」，两者不通用。");
    eprintln!("⚠️ 重新编译 firectl 后要重新签名（`codesign --force --sign <证书> --identifier");
    eprintln!("   com.tankxu.firectl <路径>`），否则签名一变授权会静默失效。发布版已签好。");
    eprintln!("⚠️ 别用 sudo —— root 不在你的图形登录会话里，反而连设备都打不开。");
    eprintln!("（日常使用不受影响：FireVibe.app 有自己的授权，装好即用。）");
}

fn run_cli() -> anyhow::Result<()> {
    #[cfg(target_os = "macos")]
    become_self_responsible();
    let args: Vec<String> = std::env::args().skip(1).collect();
    let has = |f: &str| args.iter().any(|a| a == f);
    let val = |f: &str| {
        args.iter()
            .position(|a| a == f)
            .and_then(|i| args.get(i + 1))
            .cloned()
    };

    // 未知选项直接报错 —— 别让拼错的命令（比如 --proble-all）静默掉进默认引擎模式
    // （那会加载 app 配置、跑起来，用户还以为在跑自己想要的命令）。
    const KNOWN: &[&str] = &[
        "--help", "-h", "--list-devices", "--exclusive", "--device", "--mode", "--gain",
        "--no-voice", "--descriptor", "--probe-all", "--probe-mic", "--keys", "--mic-off-test", "--mic-probe",
        "--adapt", "--map", "--sniff", "--tap", "--watch-mods", "--modcmp", "--mic", "--hold",
        "--run", "--type", "--inputs", "--set-input", "--config", "--battery", "--hid-list",
        "--loopback-test", "--feed-tone", "--pin-input", "--all", "--collection-test", "--mic-listen", "--no-cmd", "--secs", "--mic-check", "--ir-blast",
    ];
    if let Some(bad) = args.iter().find(|a| {
        a.starts_with("--")
            && !KNOWN.contains(&a.as_str())
            // 带值的参数按前缀放行，如 --secs=30
            && !KNOWN.iter().any(|k| a.starts_with(&format!("{k}=")))
    }) {
        eprintln!("未知选项：{bad}");
        // 用编辑距离挑最接近的做「是不是想输」提示
        fn edit_dist(a: &str, b: &str) -> usize {
            let (a, b): (Vec<char>, Vec<char>) = (a.chars().collect(), b.chars().collect());
            let mut prev: Vec<usize> = (0..=b.len()).collect();
            for (i, ca) in a.iter().enumerate() {
                let mut cur = vec![i + 1];
                for (j, cb) in b.iter().enumerate() {
                    let cost = if ca == cb { 0 } else { 1 };
                    cur.push((prev[j + 1] + 1).min(cur[j] + 1).min(prev[j] + cost));
                }
                prev = cur;
            }
            prev[b.len()]
        }
        if let Some(g) = KNOWN.iter().filter(|k| k.starts_with("--")).min_by_key(|k| edit_dist(k, bad)) {
            if edit_dist(g, bad) <= 3 {
                eprintln!("是不是想输：{g} ？");
            }
        }
        eprintln!("跑 `firectl --help` 看全部选项。");
        std::process::exit(2);
    }

    if has("--help") || has("-h") {
        println!(
            "firectl [选项]\n\
             \x20 --list-devices     列出音频输出设备\n\
             \x20 --exclusive        独占打开设备（系统收不到原始按键）\n\
             \x20 --device <前缀>    覆盖输出设备\n\
             \x20 --mode gate|always 覆盖送流方式\n\
             \x20 --gain <倍数>      覆盖增益\n\
             \x20 --no-voice         只测按键\n\
             \x20 --descriptor       打印 HID 报告描述符后退出\n\
             \x20 --mic-probe        开麦载荷探测：在已知命令通道 0xF2 上逐个试，看哪个能起流\n\
             \x20 --probe-all        ★换遥控器就跑这个：一条命令走完全套，生成报告文件\n\
             \x20 --adapt            只做「选设备 + 逐键认键」（--probe-all 的子集）\n\
             \x20 --probe-mic        旧名，等同 --probe-all\n\
             \x20 --hid-list         列出所有 HID 设备的 VID/PID\n\
             \x20 --map              按键测绘：逐个记录每个物理键的真实 HID usage\n\
             \x20 --sniff            原始 report 嗅探：打印每一条报文（含 vendor 0xEF/0xF1）\n\
             \x20 --keys             按键边沿追踪：每个键的按下/松开带时间戳，诊断长按瞬断\n\
             \x20 --tap              看系统把遥控器按键翻译成了什么事件（只打印非字符键）\n\
             \x20 --mic              强制开麦并送流进虚拟声卡，实时看电平。\n\
             \x20                    只测音频链路，完全不看按键怎么配的。\n\
             \x20 --mic --hold <键>  同时按住一个快捷键（模拟「按住说话 + 触发第三方工具」）\n\
             \x20 --run <位置>:<short|long>  直接执行某个位置配置好的动作，比如 mic:long\n\
             \x20 --inputs           列出可用输入设备并标出当前默认\n\
             \x20 --set-input <前缀> 切换系统默认输入设备\n\
             \n\
             权限（重要）：\n\
             \x20 · 读写遥控器需要「输入监控」权限。firectl 会把自己声明为独立授权主体，\n\
             \x20   所以把 **firectl 本体**加进列表（不是终端），加一次任何终端都认。\n\
             \x20   系统设置 › 隐私与安全性 › 输入监控 → 点「+」→ 选中 firectl → 打开开关。\n\
             \x20 · 要的是「输入监控」，不是「辅助功能」。\n\
             \x20 · 首次报 not permitted 是正常的：照上面加一次即可，之后不再问。\n\
             \x20 · 别用 sudo —— root 不在图形会话里，反而打不开设备。"
        );
        return Ok(());
    }

    // 「按键能用但没声音」专用：一条命令把判定音频通路需要的信息全打出来。
    // 两步：先被动看按住麦克风键发什么，再主动发一次开麦命令。
    // ⚠️ 只发 0xF2 的两个已知取值（0x01 开 / 0x00 关），**不做 opcode 盲扫** ——
    // 对不明设备盲扫厂商命令可能干出不可逆的事。
    // 老命令名，直接转给 --probe-all —— 麦克风那几步已经并进去了。
    // 留两份实现必然跑偏（上一次就是 --probe-mic 少发了开麦，把结论测反了）。
    if has("--probe-mic") {
        println!("（--probe-mic 已并入 --probe-all，直接往下跑）");
        return probeall::run();
    }

    // 换一款遥控器的唯一入口：一条命令走完全套并落一份报告文件
    if has("--mic-off-test") {
        return mic_off_test();
    }

    if has("--mic-probe") {
        return mic_probe();
    }


    if has("--collection-test") {
        return collection_test();

    }



    if has("--mic-listen") {
        return mic_listen();


    }




    if has("--mic-check") {
        return mic_check();



    }





    if has("--ir-blast") {
        let rest: Vec<String> = args.iter().skip_while(|a| *a != "--ir-blast").skip(1).cloned().collect();
        return ir_blast_cmd(&rest);




    }

    if has("--keys") {
        return key_trace();
    }

    if has("--probe-all") {
        return probeall::run();
    }

    // 换一款遥控器：选设备 → 逐键认键 → 看报文，全程写进配置
    if has("--adapt") {
        return adapt::run();
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
    // 验证「电量能不能主动读」：0x03 是 1 字节 INPUT 报文（电池强度 0-100），
    // 设备想发才发，所以界面常常空着。HID 允许 GetReport(Input, id) 主动取。
    if has("--battery") {
        let cfg = firevibe_core::config::Config::load();
        let (vid, pid) = cfg.device_ids();
        let api = hidapi::HidApi::new()?;
        let dev = api.open(vid, pid)?;
        println!("设备 0x{vid:04x}/0x{pid:04x} 已打开\n");
        for (label, len) in [("2 字节缓冲", 2usize), ("8 字节缓冲", 8), ("64 字节缓冲", 64)] {
            let mut buf = vec![0u8; len];
            buf[0] = 0x03; // 要读的 report id
            match dev.get_input_report(&mut buf) {
                Ok(n) => {
                    println!("  {label}: 读到 {n} 字节 → {:02x?}", &buf[..n.min(len)]);
                    // 约定：buf[0] 是 report id，后面才是数据
                    if n >= 2 {
                        println!("      电量 = {}%", buf[1]);
                    } else if n == 1 {
                        println!("      只回了 1 字节，可能就是电量本身 = {}%", buf[0]);
                    }
                }
                Err(e) => println!("  {label}: 失败 {e}"),
            }
        }
        return Ok(());
    }

    if has("--hid-list") {
        let (want_vid, want_pid) = firevibe_core::config::Config::load().device_ids();
        let api = hidapi::HidApi::new()?;
        println!("{:<8} {:<8}  {:<28} {}", "VID", "PID", "产品名", "厂商");
        println!("{}", "-".repeat(72));
        // --all：不按 VID/PID 去重，把每个 top-level collection 都列出来。
        // macOS 会把一份多 Application Collection 的报告描述符拆成好几个 IOHIDDevice
        // （VID/PID 完全一样），而音频报告 0xF0 和开麦命令 0xF2 只在 Vendor FF00 那个上。
        // hidapi 的 open(vid,pid) 取枚举里第一个 —— 挑错了写入照样成功但收不到音频。
        let show_all = has("--all");
        let mut seen = std::collections::HashSet::new();
        for d in api.device_list() {
            if !show_all && !seen.insert((d.vendor_id(), d.product_id())) {
                continue;
            }
            let ours = d.vendor_id() == want_vid && d.product_id() == want_pid;
            if show_all {
                println!(
                    "0x{:04x}   0x{:04x}    usage_page 0x{:04x} usage 0x{:02x}  iface {:>2}  {:<20} {}",
                    d.vendor_id(),
                    d.product_id(),
                    d.usage_page(),
                    d.usage(),
                    d.interface_number(),
                    d.product_string().unwrap_or("?"),
                    if ours { "← 目标 VID/PID" } else { "" }
                );
                continue;
            }
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
    let no_voice = has("--no-voice");

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

    if !no_voice {
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
                Event::KeyEdge { key, down } => {
                    println!("  {} {}", if down { "边沿↓" } else { "边沿↑" }, key)
                }
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


