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
//! 所以这套只适合电视/音响那类短码（NEC、RC5 之类）。空调（比如 Daikin 经典
//! ARC 每次发 3 帧、帧间隔 25~35 ms）塞不进来 —— 详见
//! `~/LocalDev/firetv-remote-mac/daikin-capture-spec.md`。

use serde::{Deserialize, Serialize};

/// 设备侧对单个时长的上限（有符号 int16）
pub const MAX_DURATION_US: i32 = 32767;
/// 设备侧最多接受几段原始码
pub const MAX_SEQUENCES: usize = 2;

/// 一条红外码。字段名和抓码侧的 JSON 对齐，用户直接粘贴即可。
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
        let code: IrCode =
            serde_json::from_str(s).map_err(|e| format!("JSON 解析失败：{e}"))?;
        code.validate()?;
        Ok(code)
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
