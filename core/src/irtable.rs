//! 仿品遥控器（PID 0x0425）的红外：**烧一张键位表进去**，不是即时发射。
//!
//! # 为什么和原厂那条路不一样
//!
//! 原厂（0x0421）实现了 blast：主机把码写进 `FE151503`，遥控器立刻打一发，
//! 一次性、不留痕。仿品**没实现 blast**，但实现了 `FE151501` MAPPING ——
//! 电视的「设备控制」就是这么给它写电视机红外码的（反编译 + logcat 实证，
//! 见 `~/LocalDev/firetv-remote-mac/NOTES.md`）。
//!
//! 于是仿品这边的语义是反过来的：
//!
//! | | 原厂 0x0421 | 仿品 0x0425 |
//! |---|---|---|
//! | 谁发起 | 电脑（按键 → app → 蓝牙 → 发射） | **遥控器自己**（按实体键即发） |
//! | 电脑关着 | 不发 | **照发** |
//! | 能绑几个键 | 不限（不经过按键） | **只有 4 个**（见 [`scan_id`]） |
//! | 长按槽 | 可以 | 不行（表里没有长按这回事） |
//! | 生效时机 | 立刻 | 要先把表写进遥控器（十几秒） |
//!
//! # 表长什么样
//!
//! ```text
//! [u8 行数]
//!   行: [u8 scanId][u8 动作数][u16-le 动作区字节数][动作…]
//!     动作: [u8 type][u8 flags][u16-le 载荷长度][载荷]
//! ```
//!
//! # 两条实测出来的硬规矩
//!
//! **① 每行必须正好两个动作。** 红外行是「红外 + `BLE_KEYPRESS`」；
//! 没有红外的行用「`NO_ACTION` + `BLE_KEYPRESS`」占位，**不能只写一个动作**。
//!
//! **② 四行都得在。** 只写一行（哪怕那一行是完整的两个动作）也不生效。
//!
//! 两条都是抓码实测出来的，因为**违反了设备照样回 `0x02` 说成功**：
//!
//! | 表 | 行数 | 每行 | 抓码结果 |
//! |---|---|---|---|
//! | 电视原表改一行 | 4 | 都是 红外+按键 | ✅ 发我们的码 |
//! | 三行只有按键 | 4 | 3 行 1 个动作 | ❌ 发的还是旧码 |
//! | 只留静音一行 | 1 | 红外+按键 | ❌ 发的还是旧码 |
//! | `NO_ACTION` 占位 | 4 | 都是 2 个动作 | ✅ 发我们的码 |
//!
//! ⚠️ 所以**永远不要拿回执当验证**。这一路上错单位、缺 `BLE_KEYPRESS`、
//! 行数不对、动作数不对，设备一律回 `0x02`。只有抓码能判对错。

use crate::ir::IrCode;
use crate::layout::Slot;

/// `KeyMapActionType.IR_OPT`。电视给 Power 用 6、给音量/静音用 3，两个都能用；
/// 我们统一用 6，因为**实测抓码验证过的是 6**。
const ACTION_IR_OPT: u8 = 6;
/// `KeyMapActionType.BLE_KEYPRESS`。载荷固定一个 `0x00`。
const ACTION_BLE_KEYPRESS: u8 = 5;
/// `KeyMapActionType.NO_ACTION`，零长度载荷。**没有红外的行拿它占位**——
/// 见下面「每行必须正好两个动作」。
const ACTION_NONE: u8 = 0;

/// **单个红外动作**的载荷字节上限 —— 保守值，见下面为什么。
///
/// ❗ **真实上限还没搞清楚。** 已知的三个数据点自相矛盾：
///
/// | 载荷 | 表的形状 | 抓码结果 |
/// |---|---|---|
/// | 168 B（67 脉冲） | 三行 `NO_ACTION` 占位 | ✅ 发我们的码 |
/// | 173 B（68 脉冲） | 电视自己写的四行 | ✅（电视天天在用） |
/// | **561 B（263 脉冲）** | **三行 `NO_ACTION` 占位** | **❌ 发兜底乱码** |
/// | 561 B（263 脉冲） | 电视那张、四行都带真码 | ✅ 发我们的码 |
///
/// 同一条码，在**更大**的表里成功、在**更小**的表里失败 —— 所以不是简单的
/// 「载荷 ≤ N」，还和表的形状有关，机制未明。
///
/// 早先按 561 放行，结果是**界面显示绿色通过、写进去却发一条毫不相干的码**
/// （NEC 地址 `027D`，遥控器的兜底码）—— 那可能误控别的电器，比拦住更糟。
/// 所以现在压到只在**验证过的区间**内放行。
///
/// 要放宽的话：先二分找到真边界并**抓码确认**，别只看写入回执 —— 超限时
/// 设备照样回 `0x02` 说成功。
pub const MAX_PAYLOAD_BYTES: usize = 180;

/// 单条码最多多少个脉冲（由 [`MAX_PAYLOAD_BYTES`] 换算，留出头部余量）
pub const MAX_PULSES: usize = 72;

/// 能挂红外的物理键 → 固件的 scanId。
///
/// ⚠️ **只有这四个是实证的**：从 Fire TV 的 `FULL_SYNC` 载荷里读出来的
/// （`TableUpdate.STARK` 下就 Power / VolumeUp / VolumeDown / Mute 四项），
/// 并且逐个抓码验证过。
///
/// 完整的 scanId 表在电视侧的 `KeyMapProductConfig`（日志只说
/// 「Found existing Scan ID Map for pid 0x0425」，不打印内容），没拿到。
/// 别猜别的键的 scanId —— 猜错了写进去是**静默无效**，设备照样回 0x02。
pub fn scan_id(slot: Slot) -> Option<u8> {
    Some(match slot {
        Slot::Power => 2,
        Slot::VolUp => 6,
        Slot::VolDown => 9,
        Slot::Mute => 18,
        _ => return None,
    })
}

/// 能挂红外的键，按表里的顺序
pub const IR_SLOTS: [Slot; 4] = [Slot::Power, Slot::VolUp, Slot::VolDown, Slot::Mute];

/// 这个键能不能挂红外
pub fn supports_ir(slot: Slot) -> bool {
    scan_id(slot).is_some()
}

/// 这条码放不放得进仿品遥控器。放不下就返回一句能直接显示给用户的话。
///
/// 抽出来是为了**在输入框里就判**：不然用户粘一条空调码，看到绿色的
/// 「码没问题」，等到保存才被拦 —— 而在更早的版本里连拦都没有，
/// 直接写进去，那个键从此发一条乱码。
pub fn check_code(code: &IrCode) -> Result<(), String> {
    let payload = code.compile_payload()?;
    if payload.len() <= MAX_PAYLOAD_BYTES {
        return Ok(());
    }
    let pulses: usize = code.sequences.iter().map(Vec::len).sum();
    Err(format!(
        "这条码放不进仿品遥控器：{pulses} 个脉冲，目前只支持到 {MAX_PULSES} 个。\
         电视 / 机顶盒这类一帧的码（NEC、Samsung 大多是 67~68 个脉冲）没问题；\
         再长的目前会被遥控器丢掉、改发一条乱码，所以先拦住。\
         长码请用原厂遥控器 —— 它走的是另一条通路，空调整帧都发得出去。"
    ))
}

fn action(kind: u8, flags: u8, payload: &[u8]) -> Vec<u8> {
    let mut v = vec![kind, flags];
    v.extend_from_slice(&(payload.len() as u16).to_le_bytes());
    v.extend_from_slice(payload);
    v
}

/// 拼一张完整的键位表。
///
/// `want` 里给哪个键配了码就用哪条，没给的键自动占位。
///
/// **四行永远都写、每行永远两个动作** —— 这不是洁癖，是设备的硬要求（见模块注释）。
/// 顺带的好处：用户把动作删掉，下一次同步就真的把那个键的红外清了，
/// 不会在遥控器里留着旧码。
pub fn build(want: &[(Slot, Option<&IrCode>)]) -> Result<Vec<u8>, String> {
    for (slot, _) in want {
        if !supports_ir(*slot) {
            return Err(format!("{slot:?} 这个键不能挂红外"));
        }
    }
    let mut out = vec![IR_SLOTS.len() as u8];
    for slot in IR_SLOTS {
        let sid = scan_id(slot).expect("IR_SLOTS 里的键一定有 scanId");
        let code = want.iter().find(|(s, _)| *s == slot).and_then(|(_, c)| *c);
        let mut blob = match code {
            Some(c) => {
                check_code(c)?;
                action(ACTION_IR_OPT, c.repeat_flags(), &c.compile_payload()?)
            }
            None => action(ACTION_NONE, 0, &[]),
        };
        blob.extend_from_slice(&action(ACTION_BLE_KEYPRESS, 0, &[0]));
        out.extend_from_slice(&[sid, 2]);
        out.extend_from_slice(&(blob.len() as u16).to_le_bytes());
        out.extend_from_slice(&blob);
    }
    Ok(out)
}

/// 四行全是占位、不发任何红外的表 —— 用来把遥控器上烧过的码清掉。
/// 键照常起作用（`BLE_KEYPRESS` 还在），只是不再打红外。
pub fn build_clear() -> Vec<u8> {
    build(&[]).expect("空表不可能超长")
}

/// 表的十六进制，直接喂给 irblast 小进程
pub fn to_hex(table: &[u8]) -> String {
    table.iter().map(|b| format!("{b:02x}")).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn code(pulses: usize) -> IrCode {
        IrCode {
            label: "t".into(),
            frequency: 38000,
            duty_cycle: 33,
            sequences: vec![vec![560; pulses]],
            repeat: 0,
            post_delay_ms: 0,
            toggle_bitmask: 0,
            repeat_type: "basic".into(),
        }
    }

    /// 解表 —— 和 `firetv-remote-mac/surg.py` 同一套，用来自检
    fn parse(b: &[u8]) -> Vec<(u8, Vec<(u8, Vec<u8>)>)> {
        let mut o = 1;
        let mut rows = Vec::new();
        for _ in 0..b[0] {
            let (sid, n) = (b[o], b[o + 1]);
            let alen = u16::from_le_bytes([b[o + 2], b[o + 3]]) as usize;
            o += 4;
            let blob = &b[o..o + alen];
            o += alen;
            let mut p = 0;
            let mut acts = Vec::new();
            while p < blob.len() {
                let kind = blob[p];
                let ln = u16::from_le_bytes([blob[p + 2], blob[p + 3]]) as usize;
                p += 4;
                acts.push((kind, blob[p..p + ln].to_vec()));
                p += ln;
            }
            assert_eq!(acts.len(), n as usize, "动作数对不上");
            rows.push((sid, acts));
        }
        assert_eq!(o, b.len(), "解析没吃完整张表");
        rows
    }

    #[test]
    fn 空表四行都在_每行都是占位加按键() {
        let t = build_clear();
        let rows = parse(&t);
        assert_eq!(rows.len(), 4);
        assert_eq!(rows.iter().map(|r| r.0).collect::<Vec<_>>(), vec![2, 6, 9, 18]);
        for (_, acts) in &rows {
            // 只写一个动作设备不认（实测），必须占位凑满两个
            assert_eq!(acts.len(), 2);
            assert_eq!(acts[0].0, ACTION_NONE);
            assert!(acts[0].1.is_empty());
            assert_eq!(acts[1].0, ACTION_BLE_KEYPRESS);
            assert_eq!(acts[1].1, vec![0]);
        }
    }

    #[test]
    fn 有码的行是红外加按键_而且顺序不能反() {
        let c = code(67);
        let t = build(&[(Slot::Mute, Some(&c))]).unwrap();
        let rows = parse(&t);
        // 行顺序固定 2/6/9/18，静音是最后一行
        let mute = rows.iter().find(|r| r.0 == 18).unwrap();
        assert_eq!(mute.1.len(), 2);
        // 红外在前、BLE_KEYPRESS 在后 —— 电视就是这个顺序
        assert_eq!(mute.1[0].0, ACTION_IR_OPT);
        assert_eq!(mute.1[1].0, ACTION_BLE_KEYPRESS);
        // 没配码的行也得是两个动作
        let pw = rows.iter().find(|r| r.0 == 2).unwrap();
        assert_eq!(pw.1.len(), 2);
        assert_eq!(pw.1[0].0, ACTION_NONE);
    }

    #[test]
    fn 载荷是十微秒一格() {
        let c = code(2);
        let t = build(&[(Slot::Mute, Some(&c))]).unwrap();
        let rows = parse(&t);
        let payload = &rows.iter().find(|r| r.0 == 18).unwrap().1[0].1;
        // 头部之后是 int16-le 的「格」，560 µs → 56 格
        let ticks = &payload[payload.len() - 4..];
        assert_eq!(i16::from_le_bytes([ticks[0], ticks[1]]), 56);
    }

    /// 超限**不是**干净的失败：设备照样回 0x02，然后那个键发出一条毫不相干的
    /// 乱码（实测两次都是 NECext `027D4CB3`）。所以必须在这儿拦住。
    #[test]
    fn 超长要报错而不是写进去变成乱码() {
        let big = code(263); // 实测这个长度会被丢掉、改发兜底乱码
        let e = build(&[(Slot::Mute, Some(&big))]).unwrap_err();
        assert!(e.contains("263 个脉冲"), "报错要说清超了多少：{e}");
        // 界面上是用 check_code 提前判的，两条路必须给同一个答案
        assert!(check_code(&big).is_err());
        // 67 脉冲（典型 NEC / Samsung 电视码）是实测能过的那一档
        assert!(check_code(&code(67)).is_ok());
        assert!(build(&[(Slot::Mute, Some(&code(67)))]).is_ok());
    }

    /// 上限卡的是**单个动作**，所以四个键各挂一条长码也没问题 ——
    /// 这一点是实测的（726 字节的小表里塞 673 字节的码照样被拒）。
    #[test]
    fn 四个键各挂一条最长的码也行() {
        let c = code(MAX_PULSES);
        let entries: Vec<_> = IR_SLOTS.iter().map(|s| (*s, Some(&c))).collect();
        let t = build(&entries).unwrap();
        assert_eq!(parse(&t).len(), 4);
        assert!(t.len() > 700, "四条码应该是张不小的表：{} 字节", t.len());
    }

    #[test]
    fn 不能挂红外的键要挡住() {
        let c = code(10);
        assert!(build(&[(Slot::Home, Some(&c))]).is_err());
        assert!(!supports_ir(Slot::Home));
        assert!(supports_ir(Slot::Mute));
    }

    /// **跨实现对拍**：拿 `firetv-remote-mac/surg.py` 那套编码当黄金样本。
    ///
    /// 为什么值得钉死：这串字节**在真机上抓码验证过** —— 写进遥控器、按静音键，
    /// StackChan 收到 NEC 32 位、67 个脉冲，和写下去的逐位一致。
    /// 而且它就是「三行占位 + 一行红外」那张表 —— 占位那条规矩也一起钉死了。
    /// 这里的 Rust 是重写，重写就可能偏。任何一个字节对不上都说明格式跑偏了，
    /// 而跑偏在设备上是**静默失败**（照样回 0x02，就是不发射），只靠跑真机很难发现。
    #[test]
    fn 和真机验证过的编码器逐字节一致() {
        // DynaScan DS2 · Power，库里第一条 NEC POWER —— 和当初真机测的同一条
        let hit = &crate::irdb::search("dynascan ds2", 1)[0];
        let btns = crate::irdb::buttons_of(hit.idx);
        let i = btns.iter().position(|(n, _)| n.eq_ignore_ascii_case("Power")).unwrap();
        let c = crate::irdb::code_of(hit.idx, i).unwrap();
        assert_eq!(c.sequences[0].len(), 67, "码变了，对拍样本失效");

        let got = to_hex(&build(&[(Slot::Mute, Some(&c))]).unwrap());
        const GOLDEN: &str = "040202090000000000050001000006020900000000000500010000090209000000000005000100001202b1000600a800665b33383030305d635b33335d6c5b36375d5b305d725b305d645b305d745b305d008403c2013800a9003800380038003800380038003800a9003800380038003800380038003800a900380038003800380038003800380038003800a90038003800380038003800a900380038003800380038003800380038003800a9003800380038003800380038003800a9003800a9003800a9003800a900380038003800a9003800a90038000500010000";
        assert_eq!(got, GOLDEN, "和真机验证过的字节对不上");
    }
}
