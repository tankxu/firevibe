//! Fire TV 遥控器的 HID 协议常量与报告解析。
//! 全部字节来自实机逆向，细节见 ~/LocalDev/firetv-remote-mac/NOTES.md

pub const VID: u16 = 0x0171;
pub const PID: u16 = 0x0421;

pub const RID_KEYBOARD: u8 = 0x01;
pub const RID_CONSUMER: u8 = 0x02;
pub const RID_BATTERY: u8 = 0x03;
pub const RID_AUDIO: u8 = 0xF0;
pub const RID_VENDOR_EF: u8 = 0xEF;
pub const RID_VENDOR_F1: u8 = 0xF1;
pub const RID_CMD: u8 = 0xF2;

/// 开麦 / 关麦。hidapi 把 buffer[0] 当 report id。
///
/// ★ 实测定死（`firectl --mic-off-test`，授权环境、控制变量、可复现）：
///   开麦 `[F2,01,01]`(3B) → 起流 ~50 帧/秒（第 3 字节 01=开）
///   关麦 **`[F2,00]`(2B)** → 停流。⚠️ **`[F2,01,00]`(3B) 停不了**（写成功但流照走 61/秒）——
///   费电真凶就是它：v0.1.0 一直用 [F2,01,00] 当关麦，从来没真关上过。
///   关麦不是「把开麦第 3 字节改 0」，而是另一条 2 字节命令。
///
/// ⚠️ 能不能发出去取决于**进程有没有「输入监控」授权**（跟长度无关）：
///   没授权 → 任何写都 `0xE00002E2 not permitted`。FireVibe.app 有自己的授权；
///   firectl 靠 disclaim 自持授权（见 main.rs）。别把权限错误当成字节错误
///   —— 我在这上面栽过一整轮。
pub const MIC_ON: [u8; 3] = [RID_CMD, 0x01, 0x01];
pub const MIC_OFF: [u8; 2] = [RID_CMD, 0x00];

use crate::keys::{Key, PAGE_CONSUMER, PAGE_KEYBOARD, PAGE_VENDOR};

/// 一台 HID 设备的身份信息，给「换一款遥控器」的选择列表用
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HidDev {
    pub vid: u16,
    pub pid: u16,
    pub product: String,
    pub vendor: String,
}

impl HidDev {
    /// 界面上显示的名字，产品名拿不到就退回标识
    pub fn label(&self) -> String {
        if self.product.trim().is_empty() {
            format!("0x{:04x}:0x{:04x}", self.vid, self.pid)
        } else {
            self.product.clone()
        }
    }
    pub fn ids(&self) -> String {
        format!("0x{:04x} / 0x{:04x}", self.vid, self.pid)
    }
}

/// 全进程唯一的 hidapi 句柄。
///
/// ## ⚠️ 绝对不要再写 `HidApi::new()`
///
/// hidapi 在 macOS 上是**全局单例**：`new()` 走 `hid_init()` 建一个进程级的
/// `IOHIDManager`，实例 drop 就 `hid_exit()` 把它释放掉。而我们打开的
/// `HidDevice` 活在读线程里、**跨越**了这个周期 —— 每次重连都 new 一个、
/// 用完就 drop，等于把 manager 反复拆了又建；旧 manager 一释放，下一次
/// `IOHIDManagerSetDeviceMatchingMultiple` 的 deviceAdded 回调就会去
/// `IOHIDDeviceScheduleWithRunLoop` 一个已经坏掉的 CF 对象，PAC 校验当场
/// 打死进程（`EXC_BREAKPOINT`，栈顶 `__CFCheckCFInfoPACSignature`）。
///
/// 设备一直在线时几乎撞不上，所以潜伏了很久；**遥控器一断开、重连每 1.5 秒
/// 枚举一次，就是每 1.5 秒抽一次奖** —— 2026-09-14 03:13 就是这么没的
/// （03:12 断开、一分钟后崩溃，日志和 .ips 时间对得上）。
///
/// 要重新枚举**只用 `reset_devices()` + `add_devices(vid, pid)`**，不要新建、
/// 也别用 `refresh_devices()` —— 后者是 `add_devices(0, 0)`，会让 hidapi 给
/// `IOHIDManagerSetDeviceMatching` 传 NULL，把系统里每一个 HID 设备都
/// schedule 进 run loop。那正是上面那个 PAC 崩溃的触发面（见 runtime::start）。
static API: parking_lot::Mutex<Option<hidapi::HidApi>> = parking_lot::Mutex::new(None);

/// **必须在 `main()` 里、主线程上、任何后台线程碰 hidapi 之前调一次。**
///
/// hidapi 的 `hid_init()` 是这么写的（hidapi/mac/hid.c::init_hid_manager）：
///
/// ```c
/// hid_mgr = IOHIDManagerCreate(...);
/// IOHIDManagerScheduleWithRunLoop(hid_mgr, CFRunLoopGetCurrent(), kCFRunLoopDefaultMode);
/// ```
///
/// —— 全局 manager 被**绑死在「谁先调用它」那个线程的 run loop** 上。而 `start()`
/// 跑在 `start_runtime` spawn 出来的**临时线程**里：线程一结束，run loop 就没了，
/// manager 却还记着它。之后任何一次枚举只要匹配到设备，`__IOHIDManagerDeviceAdded`
/// 就会往那个**已销毁的 run loop** 上 `CFRunLoopAddSource` → PAC 校验失败、
/// 进程当场被打死（`EXC_BREAKPOINT`，栈顶 `__CFCheckCFInfoPACSignature`）。
///
/// 所以在主线程先把它初始化掉：主线程的 run loop 和进程同寿，永远不会失效。
///
/// ⚠️ 顺序是关键：这一步必须**早于**任何后台线程调 `hid_api()`／`list_hid()`，
/// 否则 init 又会落到别的线程上。放 `main()` 第一屏。
pub fn warm_up_hid_api() {
    match hid_api() {
        Ok(_) => eprintln!("[firevibe] hidapi 已在主线程初始化（run loop 与进程同寿）"),
        Err(e) => eprintln!("[firevibe] hidapi 初始化失败：{e:#}"),
    }
}

pub fn hid_api() -> anyhow::Result<parking_lot::MappedMutexGuard<'static, hidapi::HidApi>> {
    use anyhow::Context;
    let mut g = API.lock();
    if g.is_none() {
        *g = Some(hidapi::HidApi::new().context("hidapi 初始化失败")?);
    }
    Ok(parking_lot::MutexGuard::map(g, |o| o.as_mut().expect("刚填过")))
}

/// 列出系统里所有 HID 设备（按 VID/PID 去重）。
///
/// ⚠️ **必须在后台线程调用** —— hidapi 枚举会跑 run loop，在 gpui 的
/// `cx.new` / `update` 里调会撞上 `RefCell already borrowed` 直接 abort。
pub fn list_hid() -> Vec<HidDev> {
    let Ok(mut api) = hid_api() else {
        return Vec::new();
    };
    // 这里是「列出所有 HID 设备」给用户挑，只能全枚举（`refresh_devices` =
    // `add_devices(0,0)` = 匹配全部）。⚠️ 那条路会 schedule 系统里每一个 HID
    // 设备，是 PAC 崩溃的触发面 —— 所以**只有用户主动打开设备列表时才调**，
    // 绝不能放进任何轮询/重试路径（重连路径用 add_devices(vid,pid)）。
    let _ = api.refresh_devices();
    let mut seen = std::collections::HashSet::new();
    let mut out: Vec<HidDev> = api
        .device_list()
        .filter(|d| seen.insert((d.vendor_id(), d.product_id())))
        .map(|d| HidDev {
            vid: d.vendor_id(),
            pid: d.product_id(),
            product: d.product_string().unwrap_or("").trim().to_string(),
            vendor: d.manufacturer_string().unwrap_or("").trim().to_string(),
        })
        .collect();
    // 有名字的排前面，方便肉眼找遥控器
    out.sort_by(|a, b| {
        (a.product.is_empty(), a.label().to_lowercase())
            .cmp(&(b.product.is_empty(), b.label().to_lowercase()))
    });
    out
}

/// 从键盘报告里解出当前按下的键（3 字节 usage 数组）
pub fn parse_keyboard(payload: &[u8]) -> Vec<Key> {
    payload
        .iter()
        .filter(|&&u| u != 0)
        .map(|&u| Key::new(PAGE_KEYBOARD, u as u16))
        .collect()
}

/// 从 Consumer 报告里解出当前按下的键（2 个小端 u16）
pub fn parse_consumer(payload: &[u8]) -> Vec<Key> {
    payload
        .chunks_exact(2)
        .map(|c| u16::from_le_bytes([c[0], c[1]]))
        .filter(|&u| u != 0)
        .map(|u| Key::new(PAGE_CONSUMER, u))
        .collect()
}

/// 从 vendor report 0xEF 解出按下的 App 快捷键。
/// 实测：按下 `A1 00 00`（A1..A4 对应 prime/NETFLIX/Disney+/hulu），松开 `00 00 00`。
pub fn parse_vendor(payload: &[u8]) -> Vec<Key> {
    payload
        .iter()
        .filter(|&&u| u != 0)
        .map(|&u| Key::new(PAGE_VENDOR, u as u16))
        .collect()
}

// ───────────────────────── 「输入监控」授权自检 ─────────────────────────

#[cfg(target_os = "macos")]
#[link(name = "IOKit", kind = "framework")]
extern "C" {
    /// `IOHIDCheckAccess(kIOHIDRequestTypeListenEvent)`
    /// 返回 0=已授权 1=被拒 2=还没问过
    fn IOHIDCheckAccess(request_type: u32) -> u32;
}

/// 「输入监控」到底给没给。
///
/// 加它是因为这个权限的失效方式特别隐蔽：**枚举设备照常、打开设备也成功，
/// 只是一条输入报告都收不到**。表现成「设备连上了但按键没反应」，
/// 极容易误判成设备坏了 —— 实际白查了大半天。
/// 而系统设置里的开关**看着是开的**也不代表真生效（见 CLAUDE.md 的签名坑）。
///
/// ⚠️ **但它也会说谎，别把它当唯一判据**（2026-09-19 实测）：那台 Mac 的 HID 读取
/// 通路坏掉之后，**任何进程都读不到任何设备的 input report**，而这个 API 从头到尾
/// 回答「已授权」—— 连用户把系统设置里那条记录整个删掉、`tccutil reset` 跑过之后，
/// 它依然说「已授权」。**真正的判据只有「实际读到没读到」**（`firectl --linkcheck`
/// 拿内置触控板做对照）。那次的解药是重启 Mac，见 CLAUDE.md。
pub fn input_monitoring() -> &'static str {
    #[cfg(target_os = "macos")]
    unsafe {
        // kIOHIDRequestTypeListenEvent = 1
        match IOHIDCheckAccess(1) {
            0 => "已授权",
            1 => "被拒绝",
            _ => "还没问过",
        }
    }
    #[cfg(not(target_os = "macos"))]
    "不适用"
}
