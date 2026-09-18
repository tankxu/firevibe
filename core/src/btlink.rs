//! 主动断开遥控器的 BLE 链路，让它真的睡着。
//!
//! ## 为什么需要
//!
//! 遥控器**睡着 ≠ 断开**：只要链路还挂着，它作为从设备就得按 connection interval
//! 持续醒来应答，而它要的打盹许可被 `bluetoothd` 拒（latency 49 > 30）—— 这才是
//! 待机掉电的来源。链路一旦建立不会自己散，碰它一下就重新连上并一直挂着。
//! 断开之后它是**真睡**：实测一小时零广播、不会被 macOS 拽回来，按一下键就醒。
//!
//! ## 配方（三个坑，都踩过）
//!
//! 1. **`IOBluetoothDevice.closeConnection()` 是空转** —— 返回 `kIOReturnSuccess`
//!    却什么都没发生（跑 run loop 等三秒 `isConnected` 仍是 true）。那套 API 是给
//!    classic 蓝牙的，对已配对的 BLE HID 无效。`blueutil --disconnect` 同理无效。
//!    佐证：系统设置的蓝牙面板**只链接 CoreBluetooth，根本没链 IOBluetooth**。
//! 2. **必须先 `connect` 再 `cancel`** —— `cancelPeripheralConnection` 的语义是
//!    「取消**我自己**的连接」，没 connect 过就无从取消，静默无效。我先这么试了半天，
//!    据此下过「断链在用户空间是死的」的结论，**是错的**。
//! 3. 公开 API 不够时降级到私有的 `cancelPeripheralConnection:force:`（force=YES
//!    才连「不是我建立的连接」也断）。helper 里是先试公开、不行再降级。
//!
//! ## 为什么走 helper 进程
//!
//! 要用 CoreBluetooth，而 `CBCentralManager` **不能在 app 进程里建** —— 挂着的
//! 授权框会让状态永远停在 `Unknown(0)`，连本机其它进程的蓝牙一起卡住（见
//! battery.rs 顶部那段）。所以和读电量共用 battprobe：裸二进制、fork/exec 拉起、
//! TCC 归责到父进程；万一卡住也只卡这个小进程，**父进程超时杀掉**即可。

use std::time::{Duration, Instant};

/// helper 内部要 connect(2s) + 公开 cancel(3s) + 私有 cancel(4s)，给够余量
const TIMEOUT: Duration = Duration::from_secs(20);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    /// 本来连着，已经断开了
    Closed,
    /// 它本来就不在已连接列表里（已经睡了）
    Already,
}

/// battprobe 的位置。firectl 和 app 都要能找到它：
/// 先看 `FIREVIBE_PROBE`，再看自己同目录（app bundle 内），最后退回已安装的 app。
fn helper() -> Option<std::path::PathBuf> {
    if let Some(p) = crate::battery::probe_path() {
        return Some(p);
    }
    let fallback = std::path::PathBuf::from("/Applications/FireVibe.app/Contents/MacOS/battprobe");
    fallback.is_file().then_some(fallback)
}

/// 断开名字含 `name_contains` 的那台设备。
///
/// 只动名字对得上的那一台 —— 宁可不断，也不能把用户的键盘/鼠标断掉。
pub fn disconnect(name_contains: &str) -> Result<Outcome, String> {
    let name = name_contains.trim();
    if name.is_empty() {
        return Err("没有设备名，不知道断哪台".into());
    }
    let p = helper().ok_or("找不到 battprobe")?;
    let mut child = std::process::Command::new(&p)
        .arg(name)
        .arg("--disconnect")
        .spawn()
        .map_err(|e| format!("battprobe 起不来: {e}"))?;

    let t0 = Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(st)) => {
                return match st.code() {
                    Some(0) => Ok(Outcome::Closed),
                    Some(3) => Ok(Outcome::Already),
                    Some(2) => Err("蓝牙没授权或没开（CBCentralManager 状态不对）".into()),
                    Some(c) => Err(format!("没断掉（battprobe 退出码 {c}）")),
                    None => Err("battprobe 被信号打断".into()),
                };
            }
            Ok(None) => {
                if t0.elapsed() > TIMEOUT {
                    let _ = child.kill();
                    let _ = child.wait();
                    return Err("battprobe 超时（多半是蓝牙授权没给：CoreBluetooth 会静默挂住）".into());
                }
                std::thread::sleep(Duration::from_millis(100));
            }
            Err(e) => return Err(format!("等 battprobe 失败: {e}")),
        }
    }
}
