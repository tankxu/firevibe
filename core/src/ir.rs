//! 红外码：解析 / 校验 / 编译成遥控器认的字节。
//!
//! 遥控器自带红外发射管。发射不走 HID，走 **BLE GATT 的 KeyMap 服务**
//! （`FE151500`，特征 `FE151503` = BLAST）—— macOS 只对 app 隐藏 HID 服务
//! `0x1812`，这个自定义服务 CoreBluetooth 直接可达，和读电量同一条路。
//!
//! 字节格式抄自 Fire TV 固件 `BluetoothKeyMapLib` 的 `KeyMapActionIr.compileRawCode()`：
//!
//! ```text
//! ASCII: f[频率]c[占空比]l[长度0][长度1]r[重复]d[后延迟]t[翻转掩码]
//! 0x00
//! 然后每段的脉冲时长，每个一个小端 int16
//! ```
//!
//! ⚠️ 两条硬限制，都来自那份固件代码，不是我们定的：
//!   - **最多 2 段**（`for i in 0..<2`）
//!   - 时长是**有符号 int16**（`(short) data`）→ 上限 **32767 µs**
//!
//! ❗ 早先在这儿断言过「空调塞不进去」，**是错的**，别照抄：
//!   · 「2 段」限制的是 Pronto 那种 intro/repeat，**和帧数无关** —— raw 数组本来就是
//!     交替的 mark/space，多帧编成**一段连续序列**即可，帧间隔就是个长 space。
//!   · 32767 µs 卡的是**单个条目**。Daikin 帧间隔约 25~29 ms，在限内。
//!     （IRremoteESP8266 里 `kDaikin2Gap=35204` 看着超了，但那是
//!     `LeaderMark(10024)+LeaderSpace(25180)` 之和，raw 里是两个独立条目。）
//! 唯一还没验证的是遥控器的表缓冲上限（一条空调码约 1~2 KB），要等发射通路通了实测。
//! 空调那边另有做法（有状态协议，一个按键只能对一个固定状态）详见
//! `~/LocalDev/firetv-remote-mac/daikin-capture-spec.md`。

use serde::{Deserialize, Serialize};

/// 设备侧对单个时长的上限（有符号 int16）
pub const MAX_DURATION_US: i32 = 32767;
/// 设备侧最多接受几段原始码
pub const MAX_SEQUENCES: usize = 2;

/// 一条红外码。
///
/// 这个 JSON **不是什么标准**，字段名照 Fire TV 固件的参数起的，只在我们自己这儿通用。
/// 业界发布量最大的是 **Pronto hex（CCF）** —— 见 `from_pronto`，`parse` 会自动识别。
/// 其它常见格式（Broadlink base64 / ESPHome / Flipper .ir / LIRC conf）都能先转成
/// Pronto 或原始 µs 列表再进来。
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct IrCode {
    /// 备注名，只给界面看
    #[serde(default)]
    pub label: String,
    /// 载波频率 Hz，常见 38000
    pub frequency: u32,
    /// 占空比 %，常见 33
    #[serde(default = "default_duty")]
    pub duty_cycle: u32,
    /// 脉冲时长（微秒），交替 mark/space。最多两段。
    pub sequences: Vec<Vec<i32>>,
    #[serde(default)]
    pub repeat: u32,
    #[serde(default)]
    pub post_delay_ms: u32,
    #[serde(default)]
    pub toggle_bitmask: u32,
    /// `basic`（默认）/ `toggle` / `sequence`
    #[serde(default = "default_repeat_type")]
    pub repeat_type: String,
}

fn default_duty() -> u32 {
    33
}
fn default_repeat_type() -> String {
    "basic".into()
}

/// 重复标志位，对应固件里的 `REPEAT_FLAG_TOGGLE=16` / `REPEAT_FLAG_SEQUENCE=32`
fn repeat_flags(t: &str) -> u8 {
    match t.to_ascii_lowercase().as_str() {
        "toggle" => 16,
        "sequence" => 32,
        _ => 0,
    }
}

impl IrCode {
    /// 从用户粘贴的 JSON 解析。错误信息直接给界面显示，所以要说人话。
    pub fn parse(s: &str) -> Result<Self, String> {
        let s = s.trim();
        if s.is_empty() {
            return Err("还没填红外码".into());
        }
        // 自动认格式，不给用户加「格式」下拉框：
        //   · Pronto hex   —— 首词 0000 / 0100（网上码库最常见）
        //   · 原始 µs 列表  —— ESP32 抓码直出的 rawData[]、Flipper 的 data:
        //   · JSON         —— **界面上不提**，这是进阶口子：repeat / post_delay_ms /
        //     toggle_bitmask / duty_cycle 这些旋钮只有它能设，而且能显式给两段。
        //     普通用户拿到的码不是 Pronto 就是原始数组，说「我们的 JSON」只会让人困惑。
        // `{` 开头有两种可能：我们的 JSON，或者 C 数组 `{9000, 4500, …}`。
        // 先按 JSON 试，不成再按原始列表试；两边都不成就报 JSON 的错
        // —— 用户既然写了 `{`，多半是想写 JSON，那个错更有指导性。
        if s.starts_with('{') {
            match serde_json::from_str::<IrCode>(s) {
                Ok(code) => {
                    code.validate()?;
                    return Ok(code);
                }
                Err(je) => {
                    if let Ok(c) = Self::from_raw_us(s) {
                        return Ok(c);
                    }
                    return Err(format!("JSON 解析失败：{je}"));
                }
            }
        }
        let first = s.split_whitespace().next().unwrap_or("");
        if matches!(first.to_ascii_uppercase().as_str(), "0000" | "0100") {
            return Self::from_pronto(s);
        }
        Self::from_raw_us(s)
    }

    pub fn validate(&self) -> Result<(), String> {
        if self.sequences.is_empty() {
            return Err("sequences 是空的".into());
        }
        if self.sequences.len() > MAX_SEQUENCES {
            return Err(format!(
                "最多 {MAX_SEQUENCES} 段，这里有 {} 段 —— 遥控器的格式只放得下两段",
                self.sequences.len()
            ));
        }
        if self.frequency == 0 {
            return Err("frequency 不能是 0（常见 38000）".into());
        }
        for (i, seq) in self.sequences.iter().enumerate() {
            if seq.is_empty() {
                return Err(format!("第 {} 段是空的", i + 1));
            }
            if let Some(bad) = seq.iter().find(|&&v| v <= 0 || v > MAX_DURATION_US) {
                return Err(format!(
                    "第 {} 段有个时长 {bad} µs 越界 —— 必须在 1..{MAX_DURATION_US} 之间\
                     （设备侧是有符号 int16）",
                    i + 1
                ));
            }
        }
        Ok(())
    }

    /// 总时长（毫秒），界面上显示用
    pub fn total_ms(&self) -> f32 {
        self.sequences.iter().flatten().sum::<i32>() as f32 / 1000.0
    }

    /// 一行人话摘要，配好之后显示在动作卡上
    pub fn summary(&self) -> String {
        let pulses: usize = self.sequences.iter().map(|s| s.len()).sum();
        let name = if self.label.is_empty() {
            String::new()
        } else {
            format!("{} · ", self.label)
        };
        format!(
            "{name}{:.0} kHz · {} 段 · {pulses} 个脉冲 · {:.0} ms",
            self.frequency as f32 / 1000.0,
            self.sequences.len(),
            self.total_ms()
        )
    }

    /// 编译成设备要的载荷。对应固件的 `compileRawCode()`。
    ///
    /// 注意 `l[..][..]` 那两个长度**永远写两个**：不足两段时第二个写 `[0]`，
    /// 固件就是这么干的（`for i in 0..<2`，越界写 `[0]`）。
    pub fn compile_payload(&self) -> Result<Vec<u8>, String> {
        self.validate()?;
        let mut out = Vec::new();
        out.extend_from_slice(format!("f[{}]", self.frequency).as_bytes());
        out.extend_from_slice(format!("c[{}]", self.duty_cycle).as_bytes());
        out.push(b'l');
        for i in 0..MAX_SEQUENCES {
            let n = self.sequences.get(i).map(|s| s.len()).unwrap_or(0);
            out.extend_from_slice(format!("[{n}]").as_bytes());
        }
        out.extend_from_slice(format!("r[{}]", self.repeat).as_bytes());
        out.extend_from_slice(format!("d[{}]", self.post_delay_ms).as_bytes());
        out.extend_from_slice(format!("t[{}]", self.toggle_bitmask).as_bytes());
        out.push(0);
        for seq in self.sequences.iter().take(MAX_SEQUENCES) {
            for &v in seq {
                out.extend_from_slice(&(v as i16).to_le_bytes());
            }
        }
        Ok(out)
    }

    /// 重复标志，外层 `compileAction` 要用
    pub fn repeat_flags(&self) -> u8 {
        repeat_flags(&self.repeat_type)
    }
}

// ─────────────────────────── Pronto hex (CCF) ───────────────────────────

/// Pronto 每个「载波周期」单位对应的微秒数。频率 = 1000000 / (k × 该常数)。
const PRONTO_TICK_US: f64 = 0.241246;

impl IrCode {
    /// 解析 Pronto hex（CCF）。
    ///
    /// ```text
    /// 0000 006D 0022 0002 | <once 序列> <repeat 序列>
    ///   │     │    │    └─ repeat 段的脉冲对数
    ///   │     │    └────── once（首发）段的脉冲对数
    ///   │     └─────────── 载波因子 k
    ///   └───────────────── 0000 = 原始码（0100 是预定义协议，我们不支持）
    /// ```
    /// 之后每个词是一段电平的长度，单位是**载波周期数**，乘周期换成微秒。
    ///
    /// 为什么优先支持它：发布量最大的红外码格式（RemoteCentral / irdb / 厂商文档），
    /// 而且 **once + repeat 正好两段**，和遥控器固件的 `l[长度0][长度1]` 一一对应 ——
    /// 那个格式看起来本来就是照 Pronto 的形状设计的。
    pub fn from_pronto(src: &str) -> Result<Self, String> {
        let words: Vec<u32> = src
            .split_whitespace()
            .map(|w| {
                u32::from_str_radix(w.trim_start_matches("0x"), 16)
                    .map_err(|_| format!("「{w}」不是十六进制"))
            })
            .collect::<Result<_, _>>()?;
        if words.len() < 4 {
            return Err("Pronto 码至少要 4 个词".into());
        }
        if words[0] != 0 {
            return Err(format!(
                "只支持原始码（首词 0000），这条是 {:04X} —— 预定义协议那种得先转成原始码",
                words[0]
            ));
        }
        let k = words[1];
        if k == 0 {
            return Err("载波因子是 0".into());
        }
        let period = k as f64 * PRONTO_TICK_US;
        let freq = (1_000_000.0 / period).round() as u32;

        let (n_once, n_rep) = (words[2] as usize, words[3] as usize);
        let need = 4 + (n_once + n_rep) * 2;
        if words.len() < need {
            return Err(format!(
                "词数不够：头部声明 {n_once} + {n_rep} 对，需要 {need} 个词，实际 {}",
                words.len()
            ));
        }
        let to_us = |ws: &[u32]| -> Vec<i32> {
            ws.iter().map(|&w| (w as f64 * period).round() as i32).collect()
        };
        let mut sequences = Vec::new();
        if n_once > 0 {
            sequences.push(to_us(&words[4..4 + n_once * 2]));
        }
        if n_rep > 0 {
            let a = 4 + n_once * 2;
            sequences.push(to_us(&words[a..a + n_rep * 2]));
        }

        let code = IrCode {
            label: String::new(),
            frequency: freq,
            duty_cycle: default_duty(), // Pronto 不带占空比
            sequences,
            repeat: 0,
            post_delay_ms: 0,
            toggle_bitmask: 0,
            repeat_type: default_repeat_type(),
        };
        code.validate()?;
        Ok(code)
    }
}

// ───────────────────── 编译成遥控器要的「一次性发射表」 ─────────────────────

/// `KeyMapActionType.IR_CODE_RAW`
const ACTION_IR_RAW: u8 = 3;

impl IrCode {
    /// 单个动作：`[u8 类型][u8 标志][u16-le 载荷长度][载荷]`
    /// 对应固件 `KeyMapAction.compileAction()`。
    pub fn compile_action(&self) -> Result<Vec<u8>, String> {
        let payload = self.compile_payload()?;
        let mut out = vec![ACTION_IR_RAW, self.repeat_flags()];
        out.extend_from_slice(&(payload.len() as u16).to_le_bytes());
        out.extend_from_slice(&payload);
        Ok(out)
    }

    /// 一次性发射表（写进 BLAST 特征的那份）。
    ///
    /// 整张表的布局是：
    /// ```text
    /// [u8 行数][u8 scanId][u8 该行动作数][u16-le 动作字节总长][动作们…]
    /// ```
    /// 而 `compileAsBlast()` 把**第一个字节（行数）去掉** —— 所以这里直接从
    /// scanId 开始拼，不写行数。
    ///
    /// `scan_id` 是这一行挂在哪个物理键上。一次性发射不经过按键，理论上无所谓，
    /// 但格式要求有这么一个字节。
    pub fn compile_blast(&self, scan_id: u8) -> Result<Vec<u8>, String> {
        let action = self.compile_action()?;
        let mut out = vec![scan_id, 1];
        out.extend_from_slice(&(action.len() as u16).to_le_bytes());
        out.extend_from_slice(&action);
        Ok(out)
    }
}

// ─────────────────────── 原始 µs 列表（ESP32 抓码直出）───────────────────────

impl IrCode {
    /// 吃 `IRremoteESP8266` 的 `resultToSourceCode()` 直出的数组，比如
    /// `uint16_t rawData[67] = {9000, 4500, 560, 1690};`，或者光是
    /// `9000, 4500, 560, 1690`（Flipper 的 `data:` 那行空格分隔也行）。
    ///
    /// 裸列表不带载波频率，默认按 38 kHz —— 绝大多数消费级设备都是。
    /// 要别的频率就用 Pronto（自带频率）或我们的 JSON。
    pub fn from_raw_us(src: &str) -> Result<Self, String> {
        let body = src
            .rsplit('=')
            .next()
            .unwrap_or(src)
            .trim()
            .trim_end_matches(';')
            .trim()
            .trim_start_matches(['{', '['])
            .trim_end_matches(['}', ']']);
        let seq: Vec<i32> = body
            .split(|ch: char| ch == ',' || ch.is_whitespace())
            .filter(|t| !t.is_empty())
            .map(|t| {
                t.parse::<i32>()
                    .map_err(|_| format!("「{t}」不是个整数微秒值"))
            })
            .collect::<Result<_, _>>()?;
        if seq.is_empty() {
            return Err("没解析出任何时长".into());
        }
        let code = IrCode {
            label: String::new(),
            frequency: 38_000,
            duty_cycle: default_duty(),
            sequences: vec![seq],
            repeat: 0,
            post_delay_ms: 0,
            toggle_bitmask: 0,
            repeat_type: default_repeat_type(),
        };
        code.validate()?;
        Ok(code)
    }

    /// 存进动作的文本形态。带上 label / 频率 / 重复这些 Pronto 和裸列表表达不了的字段。
    pub fn to_json(&self) -> String {
        serde_json::to_string(self).unwrap_or_default()
    }

    /// 反过来导成 Pronto hex —— 便携格式，可以存档或者贴到别的工具里。
    /// 和 `from_pronto` 是同一套换算，只是反着来。
    pub fn to_pronto(&self) -> String {
        let k = (1_000_000.0 / (self.frequency as f64 * PRONTO_TICK_US)).round() as u32;
        let period = k as f64 * PRONTO_TICK_US;
        let n = |i: usize| self.sequences.get(i).map(|s| s.len() / 2).unwrap_or(0);
        let mut out = format!("{:04X} {:04X} {:04X} {:04X}", 0, k, n(0), n(1));
        for seq in self.sequences.iter().take(MAX_SEQUENCES) {
            for &v in seq {
                out.push_str(&format!(" {:04X}", (v as f64 / period).round() as u32));
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn nec_power() -> IrCode {
        IrCode {
            label: "tv_power".into(),
            frequency: 38000,
            duty_cycle: 33,
            sequences: vec![vec![9000, 4500, 560, 1690]],
            repeat: 0,
            post_delay_ms: 0,
            toggle_bitmask: 0,
            repeat_type: "basic".into(),
        }
    }

    #[test]
    fn 动作头是_类型_标志_长度() {
        let a = nec_power().compile_action().unwrap();
        assert_eq!(a[0], 3, "IR_CODE_RAW");
        assert_eq!(a[1], 0, "Basic 的标志位是 0");
        let len = u16::from_le_bytes([a[2], a[3]]) as usize;
        assert_eq!(len, a.len() - 4, "长度字段要等于后面载荷的长度");
    }

    #[test]
    fn 标志位跟着重复类型走() {
        let mut c = nec_power();
        c.repeat_type = "toggle".into();
        assert_eq!(c.compile_action().unwrap()[1], 16);
    }

    #[test]
    fn blast_表不带行数字节() {
        let c = nec_power();
        let action = c.compile_action().unwrap();
        let t = c.compile_blast(7).unwrap();
        assert_eq!(t[0], 7, "scanId");
        assert_eq!(t[1], 1, "一行一个动作");
        assert_eq!(u16::from_le_bytes([t[2], t[3]]) as usize, action.len());
        assert_eq!(&t[4..], &action[..], "动作原样跟在后面");
        assert_eq!(t.len(), 4 + action.len(), "不含「行数」那个字节");
    }

    #[test]
    fn 吃_esp32_抓码直出的数组() {
        let c = IrCode::parse("uint16_t rawData[6] = {9000, 4500, 560, 1690, 560, 560};")
            .unwrap();
        assert_eq!(c.frequency, 38_000, "裸列表默认 38 kHz");
        assert_eq!(c.sequences, vec![vec![9000, 4500, 560, 1690, 560, 560]]);
    }

    #[test]
    fn 也吃裸的空格分隔_flipper那种() {
        let c = IrCode::parse("9000 4500 560 1690").unwrap();
        assert_eq!(c.sequences[0].len(), 4);
    }

    #[test]
    fn pronto_往返换算对得上() {
        let src = "0000 006D 0002 0001 0056 0015 0016 0015 0016 0400";
        let a = IrCode::from_pronto(src).unwrap();
        let b = IrCode::from_pronto(&a.to_pronto()).unwrap();
        assert_eq!(a.frequency, b.frequency);
        for (x, y) in a.sequences.iter().flatten().zip(b.sequences.iter().flatten()) {
            assert!((x - y).abs() <= 1, "往返误差过大 {x} vs {y}");
        }
    }

    #[test]
    fn 原始列表能导成_pronto() {
        let c = IrCode::parse("{9000, 4500, 560, 1690}").unwrap();
        let p = c.to_pronto();
        assert!(p.starts_with("0000 006D 0002 0000"), "{p}");
    }

    #[test]
    fn pronto_换算频率和时长() {
        // k=0x6D=109 → 周期 26.2958 µs → 38 kHz；2 对 once + 1 对 repeat
        let c = IrCode::from_pronto("0000 006D 0002 0001 0056 0015 0016 0015 0016 0400")
            .unwrap();
        assert!((c.frequency as i32 - 38028).abs() < 30, "频率 {}", c.frequency);
        assert_eq!(c.sequences.len(), 2, "once + repeat 正好两段");
        assert_eq!(c.sequences[0].len(), 4);
        assert_eq!(c.sequences[1].len(), 2);
        assert!((c.sequences[0][0] - 2261).abs() <= 1, "{:?}", c.sequences[0]);
        assert!((c.sequences[1][1] - 26927).abs() <= 2, "{:?}", c.sequences[1]);
    }

    #[test]
    fn pronto_只有_once_段() {
        let c = IrCode::from_pronto("0000 006D 0001 0000 0056 0015").unwrap();
        assert_eq!(c.sequences.len(), 1);
    }

    #[test]
    fn pronto_预定义协议要说清不支持() {
        let e = IrCode::from_pronto("0100 006D 0000 0001 0000 0000").unwrap_err();
        assert!(e.contains("原始码"), "{e}");
    }

    #[test]
    fn pronto_词数不够要报出来() {
        let e = IrCode::from_pronto("0000 006D 0004 0000 0056 0015").unwrap_err();
        assert!(e.contains("词数不够"), "{e}");
    }

    #[test]
    fn parse_自动识别_pronto_和_json() {
        assert!(IrCode::parse("0000 006D 0001 0000 0056 0015").is_ok());
        assert!(IrCode::parse(r#"{"frequency":38000,"sequences":[[560,560]]}"#).is_ok());
        assert!(IrCode::parse("{9000, 4500}").is_ok(), "C 数组也认");
        // 什么都不像 → 按原始列表报错，指出是哪个词有问题
        let e = IrCode::parse("hello world").unwrap_err();
        assert!(e.contains("hello"), "{e}");
    }

    #[test]
    fn 载荷头部按固件的格式拼() {
        let b = nec_power().compile_payload().unwrap();
        let head_end = b.iter().position(|&x| x == 0).unwrap();
        assert_eq!(
            std::str::from_utf8(&b[..head_end]).unwrap(),
            "f[38000]c[33]l[4][0]r[0]d[0]t[0]",
            "只有一段时第二个长度要写 [0]"
        );
    }

    #[test]
    fn 脉冲是小端_int16() {
        let b = nec_power().compile_payload().unwrap();
        let body = &b[b.iter().position(|&x| x == 0).unwrap() + 1..];
        assert_eq!(body.len(), 4 * 2);
        assert_eq!(&body[0..2], &9000i16.to_le_bytes());
        assert_eq!(&body[6..8], &1690i16.to_le_bytes());
    }

    #[test]
    fn 超过两段要报错() {
        let mut c = nec_power();
        c.sequences = vec![vec![100], vec![100], vec![100]];
        assert!(c.validate().unwrap_err().contains("最多 2 段"));
    }

    #[test]
    fn 时长越界要报错_有符号int16() {
        let mut c = nec_power();
        c.sequences = vec![vec![560, 40000]];
        let e = c.validate().unwrap_err();
        assert!(e.contains("40000"), "{e}");
    }

    #[test]
    fn 重复类型映射到固件的标志位() {
        let mut c = nec_power();
        assert_eq!(c.repeat_flags(), 0);
        c.repeat_type = "Toggle".into();
        assert_eq!(c.repeat_flags(), 16);
        c.repeat_type = "sequence".into();
        assert_eq!(c.repeat_flags(), 32);
    }

    #[test]
    fn 解析用户粘贴的_json() {
        let c = IrCode::parse(
            r#"{"label":"tv_power","frequency":38000,"sequences":[[9000,4500,560,1690]]}"#,
        )
        .unwrap();
        assert_eq!(c.duty_cycle, 33, "没写就用默认占空比");
        assert_eq!(c.repeat_type, "basic");
        assert!(c.summary().contains("38 kHz"));
    }

    #[test]
    fn 错误信息说人话() {
        assert!(IrCode::parse("").unwrap_err().contains("还没填"));
        assert!(IrCode::parse("{").unwrap_err().contains("JSON 解析失败"));
    }
}
