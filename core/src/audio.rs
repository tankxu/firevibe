//! 系统默认输入设备的读写。
//!
//! cpal 只能枚举设备，改不了系统默认输入 —— 那是 CoreAudio 的
//! `kAudioHardwarePropertyDefaultInputDevice`，只能直接调框架。
//!
//! ⚠️ 这些调用是同步阻塞的，而且会跑 run loop。**必须在后台线程调**，
//! 别在 gpui 的 `update`/`cx.new` 里同步调 —— 那会触发
//! 「RefCell already borrowed」（见 README 里那段）。

#[cfg(target_os = "macos")]
mod imp {
    use anyhow::{anyhow, Result};
    use core_foundation::base::TCFType;
    use core_foundation::string::{CFString, CFStringRef};
    use std::ffi::c_void;
    use std::ptr::null;

    const SYSTEM_OBJECT: u32 = 1; // kAudioObjectSystemObject

    /// 把四字符码转成 u32（CoreAudio 的 selector 都是这种）
    const fn fourcc(s: &[u8; 4]) -> u32 {
        ((s[0] as u32) << 24) | ((s[1] as u32) << 16) | ((s[2] as u32) << 8) | s[3] as u32
    }

    const DEVICES: u32 = fourcc(b"dev#");
    const DEFAULT_INPUT: u32 = fourcc(b"dIn ");
    const NAME: u32 = fourcc(b"lnam");
    const STREAMS: u32 = fourcc(b"stm#");
    const SCOPE_GLOBAL: u32 = fourcc(b"glob");
    const SCOPE_INPUT: u32 = fourcc(b"inpt");

    #[repr(C)]
    struct Addr {
        selector: u32,
        scope: u32,
        element: u32,
    }

    impl Addr {
        const fn new(selector: u32, scope: u32) -> Self {
            Self {
                selector,
                scope,
                element: 0,
            }
        }
    }

    #[link(name = "CoreAudio", kind = "framework")]
    unsafe extern "C" {
        fn AudioObjectGetPropertyDataSize(
            id: u32,
            addr: *const Addr,
            qual_size: u32,
            qual: *const c_void,
            out_size: *mut u32,
        ) -> i32;
        fn AudioObjectGetPropertyData(
            id: u32,
            addr: *const Addr,
            qual_size: u32,
            qual: *const c_void,
            io_size: *mut u32,
            out: *mut c_void,
        ) -> i32;
        fn AudioObjectSetPropertyData(
            id: u32,
            addr: *const Addr,
            qual_size: u32,
            qual: *const c_void,
            size: u32,
            data: *const c_void,
        ) -> i32;
    }

    #[derive(Clone, Debug, PartialEq)]
    pub struct InputDevice {
        pub id: u32,
        pub name: String,
    }

    fn device_name(id: u32) -> Option<String> {
        let addr = Addr::new(NAME, SCOPE_GLOBAL);
        let mut s: CFStringRef = std::ptr::null();
        let mut size = std::mem::size_of::<CFStringRef>() as u32;
        let ok = unsafe {
            AudioObjectGetPropertyData(
                id,
                &addr,
                0,
                null(),
                &mut size,
                &mut s as *mut _ as *mut c_void,
            )
        };
        if ok != 0 || s.is_null() {
            return None;
        }
        Some(unsafe { CFString::wrap_under_create_rule(s) }.to_string())
    }

    /// 这个设备有输入流吗（拿 stream 列表的字节数判断，不用解析结构体）
    fn has_input(id: u32) -> bool {
        let addr = Addr::new(STREAMS, SCOPE_INPUT);
        let mut size = 0u32;
        let ok = unsafe { AudioObjectGetPropertyDataSize(id, &addr, 0, null(), &mut size) };
        ok == 0 && size > 0
    }

    /// 所有能当输入用的设备
    pub fn input_devices() -> Vec<InputDevice> {
        let addr = Addr::new(DEVICES, SCOPE_GLOBAL);
        let mut size = 0u32;
        if unsafe { AudioObjectGetPropertyDataSize(SYSTEM_OBJECT, &addr, 0, null(), &mut size) }
            != 0
        {
            return Vec::new();
        }
        let n = size as usize / std::mem::size_of::<u32>();
        let mut ids = vec![0u32; n];
        if unsafe {
            AudioObjectGetPropertyData(
                SYSTEM_OBJECT,
                &addr,
                0,
                null(),
                &mut size,
                ids.as_mut_ptr() as *mut c_void,
            )
        } != 0
        {
            return Vec::new();
        }
        ids.into_iter()
            .filter(|&id| has_input(id))
            .filter_map(|id| device_name(id).map(|name| InputDevice { id, name }))
            .collect()
    }

    /// 当前系统默认输入设备
    pub fn default_input() -> Option<InputDevice> {
        let addr = Addr::new(DEFAULT_INPUT, SCOPE_GLOBAL);
        let mut id = 0u32;
        let mut size = std::mem::size_of::<u32>() as u32;
        let ok = unsafe {
            AudioObjectGetPropertyData(
                SYSTEM_OBJECT,
                &addr,
                0,
                null(),
                &mut size,
                &mut id as *mut _ as *mut c_void,
            )
        };
        if ok != 0 || id == 0 {
            return None;
        }
        device_name(id).map(|name| InputDevice { id, name })
    }

    /// 换系统默认输入设备
    pub fn set_default_input(id: u32) -> Result<()> {
        let addr = Addr::new(DEFAULT_INPUT, SCOPE_GLOBAL);
        let ok = unsafe {
            AudioObjectSetPropertyData(
                SYSTEM_OBJECT,
                &addr,
                0,
                null(),
                std::mem::size_of::<u32>() as u32,
                &id as *const _ as *const c_void,
            )
        };
        if ok == 0 {
            Ok(())
        } else {
            Err(anyhow!("设置默认输入设备失败，CoreAudio 返回 {ok}"))
        }
    }
}

#[cfg(not(target_os = "macos"))]
mod imp {
    use anyhow::{anyhow, Result};

    #[derive(Clone, Debug, PartialEq)]
    pub struct InputDevice {
        pub id: u32,
        pub name: String,
    }
    pub fn input_devices() -> Vec<InputDevice> {
        Vec::new()
    }
    pub fn default_input() -> Option<InputDevice> {
        None
    }
    pub fn set_default_input(_id: u32) -> Result<()> {
        Err(anyhow!("这个平台还没做输入设备切换"))
    }
}

pub use imp::{default_input, input_devices, set_default_input, InputDevice};

/// 切默认输入并**等到真的生效**。
///
/// `AudioObjectSetPropertyData` 是异步的：立刻返回，设备过几毫秒才真的变。
/// 要喂那种「收到快捷键就立刻开麦」的第三方工具（豆包、闪电说），
/// 必须先等切换落地再发快捷键 —— 否则它绑的还是你原来的麦克风，
/// 表现是工具起来了但电平一动不动。
pub fn set_default_input_and_wait(id: u32, timeout: std::time::Duration) -> bool {
    if set_default_input(id).is_err() {
        return false;
    }
    let t0 = std::time::Instant::now();
    while t0.elapsed() < timeout {
        if default_input().map(|d| d.id) == Some(id) {
            return true;
        }
        std::thread::sleep(std::time::Duration::from_millis(2));
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 切换到底要多久 —— 决定「按下麦克风键自动切、松手切回」这套可不可行。
    /// `AudioObjectSetPropertyData` 是**异步**的：它立刻返回，但默认设备
    /// 要过一会儿才真的变，所以这里轮询到生效为止。
    /// 手动跑：`cargo test -p firevibe-core -- --ignored --nocapture switch_timing`
    #[test]
    #[ignore = "会真的来回切系统输入设备"]
    fn switch_timing() {
        use std::time::{Duration, Instant};
        let orig = default_input().expect("没有默认输入设备");
        let devs = input_devices();
        let bh = devs
            .iter()
            .find(|d| d.name.to_lowercase().contains("blackhole"))
            .expect("没装 BlackHole");
        // 挑一个真麦克风（不是虚拟驱动）—— 这才是「用完切回去」的实际目标
        let other = devs
            .iter()
            .find(|d| {
                let n = d.name.to_lowercase();
                d.id != bh.id && !n.contains("blackhole") && !n.contains("virtual")
            })
            .expect("找不到真麦克风");
        println!("原始默认: {} (id {})", orig.name, orig.id);

        /// 切过去并等到真的生效，返回 (set 调用耗时, 生效总耗时)
        fn switch_and_wait(id: u32) -> (u128, u128) {
            let t = Instant::now();
            set_default_input(id).unwrap();
            let call = t.elapsed().as_micros();
            let deadline = Instant::now() + Duration::from_secs(2);
            while Instant::now() < deadline {
                if default_input().map(|d| d.id) == Some(id) {
                    break;
                }
                std::thread::sleep(Duration::from_millis(1));
            }
            (call, t.elapsed().as_micros())
        }

        let mut worst = 0u128;
        for i in 0..6 {
            let target = if i % 2 == 0 { other } else { bh };
            let (call, total) = switch_and_wait(target.id);
            worst = worst.max(total);
            println!(
                "  → {:<22} set 调用 {:>5}µs   实际生效 {:>6.1}ms",
                target.name,
                call,
                total as f64 / 1000.0
            );
        }
        set_default_input(orig.id).unwrap();
        std::thread::sleep(Duration::from_millis(200));
        println!(
            "最慢一次生效 {:.1}ms；已还原到 {}",
            worst as f64 / 1000.0,
            default_input().unwrap().name
        );
    }

    /// 手动跑：`cargo test -p firevibe-core -- --ignored --nocapture list_inputs`
    #[test]
    #[ignore = "读系统音频设备，环境相关"]
    fn list_inputs() {
        let cur = default_input();
        println!("当前默认输入: {cur:?}");
        for d in input_devices() {
            let mark = if Some(d.id) == cur.as_ref().map(|c| c.id) {
                "*"
            } else {
                " "
            };
            println!("{mark} {:>5}  {}", d.id, d.name);
        }
        assert!(!input_devices().is_empty(), "一个输入设备都没枚举到");
    }
}
