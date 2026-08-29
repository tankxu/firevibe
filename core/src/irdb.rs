//! 内置红外码库：搜设备 → 挑按键 → 拿到码。
//!
//! 数据来自 [Flipper-IRDB](https://github.com/Lucaslhm/Flipper-IRDB)，CC0-1.0
//! （公有领域），所以能直接打进包里。打包脚本见 `tools/build-irdb.py`，
//! 里面写了收哪些、丢哪些、以及为什么。
//!
//! 1605 个设备、25767 条码，gzip 后 0.99 MB。**懒加载**：用户不点搜索就不解压，
//! 解一次存住（约 8 MB 常驻，桌面应用可以接受）。

use crate::ir::IrCode;
use serde::Deserialize;
use std::sync::OnceLock;

/// 打包进二进制的库。字段名压到一个字母是为了压缩前就小一点。
const RAW: &[u8] = include_bytes!("../assets/irdb.jsonl.gz");

#[derive(Debug, Deserialize)]
struct Button {
    /// 按键名，如 `POWER` / `COOL`
    n: String,
    /// 载波频率
    f: u32,
    /// 时长序列（µs）
    s: Vec<i32>,
    /// 来源：`raw` = 库里本来就是时序；`NEC`/`NECext` = 按协议合成的
    p: String,
}

#[derive(Debug, Deserialize)]
struct Device {
    /// 分类，如 `ACs` / `TVs`
    c: String,
    /// 品牌
    b: String,
    /// 型号
    m: String,
    k: Vec<Button>,
}

/// 一条搜索结果
pub struct Hit {
    pub category: String,
    pub brand: String,
    pub model: String,
    pub buttons: usize,
    /// 在库里的下标，用 `buttons_of` 取按键
    pub idx: usize,
}

fn db() -> &'static Vec<Device> {
    static DB: OnceLock<Vec<Device>> = OnceLock::new();
    DB.get_or_init(|| {
        use std::io::Read;
        let mut s = String::new();
        if flate2::read::GzDecoder::new(RAW).read_to_string(&mut s).is_err() {
            eprintln!("[irdb] 解压失败");
            return Vec::new();
        }
        s.lines().filter_map(|l| serde_json::from_str(l).ok()).collect()
    })
}

/// 库里一共多少设备 / 多少条码
pub fn stats() -> (usize, usize) {
    let d = db();
    (d.len(), d.iter().map(|x| x.k.len()).sum())
}

/// 按品牌 / 型号 / 分类模糊搜。空串返回空 —— 一万多条全列出来没意义。
///
/// 所有关键词都要命中（空格分词），这样「daikin arc」能筛出具体型号。
pub fn search(q: &str, limit: usize) -> Vec<Hit> {
    let q = q.trim().to_lowercase();
    if q.is_empty() {
        return Vec::new();
    }
    let terms: Vec<&str> = q.split_whitespace().collect();
    let mut out = Vec::new();
    for (i, d) in db().iter().enumerate() {
        let hay = format!("{} {} {}", d.b, d.m, d.c).to_lowercase();
        if terms.iter().all(|t| hay.contains(t)) {
            out.push(Hit {
                category: d.c.clone(),
                brand: d.b.clone(),
                model: d.m.clone(),
                buttons: d.k.len(),
                idx: i,
            });
            if out.len() >= limit {
                break;
            }
        }
    }
    out
}

/// 某个设备的按键列表：(按键名, 来源标记)
pub fn buttons_of(idx: usize) -> Vec<(String, String)> {
    db()
        .get(idx)
        .map(|d| d.k.iter().map(|b| (b.n.clone(), b.p.clone())).collect())
        .unwrap_or_default()
}

/// 某个设备每个按键的脉冲数，顺序和 [`buttons_of`] 一致。
///
/// 单独给一个是因为界面要在**列按键时**就标出「这条太长、某些遥控器放不下」——
/// 为此给每个按键都 `code_of` + `compile_payload` 一遍太浪费，这里只数长度。
pub fn pulses_of(idx: usize) -> Vec<usize> {
    db()
        .get(idx)
        .map(|d| d.k.iter().map(|b| b.s.len()).collect())
        .unwrap_or_default()
}

/// 取某个按键的码，直接就是能存进动作的 `IrCode`
pub fn code_of(idx: usize, button: usize) -> Option<IrCode> {
    let d = db().get(idx)?;
    let b = d.k.get(button)?;
    let code = IrCode {
        label: format!("{} {} · {}", d.b, d.m, b.n),
        frequency: b.f,
        duty_cycle: 33,
        sequences: vec![b.s.clone()],
        repeat: 0,
        post_delay_ms: 0,
        toggle_bitmask: 0,
        repeat_type: "basic".into(),
    };
    code.validate().ok()?;
    Some(code)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn 库能解开而且不小() {
        let (devs, codes) = stats();
        assert!(devs > 1000, "设备数 {devs}");
        assert!(codes > 20000, "码数 {codes}");
    }

    #[test]
    fn 能搜到_daikin_空调() {
        let hits = search("daikin", 50);
        assert!(!hits.is_empty(), "搜不到 Daikin");
        assert!(hits.iter().any(|h| h.category.contains("AC")), "没有空调分类");
    }

    #[test]
    fn 多个关键词要同时命中() {
        let hits = search("daikin arc480a41", 10);
        assert_eq!(hits.len(), 1, "应该只剩一个型号，实际 {}", hits.len());
        assert!(hits[0].buttons >= 15, "按键数 {}", hits[0].buttons);
    }

    #[test]
    fn 取出来的码是合法的可以直接用() {
        let h = &search("daikin arc480a41", 1)[0];
        let btns = buttons_of(h.idx);
        assert!(!btns.is_empty());
        let code = code_of(h.idx, 0).expect("取不到码");
        assert!(code.validate().is_ok());
        assert!(code.compile_payload().is_ok(), "编译不出载荷");
        assert!(code.label.contains("Daikin"), "标签 {}", code.label);
    }

    #[test]
    fn 库里每条码都能编译_抽查() {
        // 全量太慢，抽 200 个设备的第一个按键
        let mut n = 0;
        for i in (0..db().len()).step_by(7).take(200) {
            if let Some(c) = code_of(i, 0) {
                assert!(c.compile_payload().is_ok(), "设备 {i} 的码编译失败");
                n += 1;
            }
        }
        assert!(n > 150, "只抽到 {n} 条");
    }

    #[test]
    fn 脉冲数和按键列表一一对应() {
        let h = &search("daikin arc480a41", 1)[0];
        let (btns, pulses) = (buttons_of(h.idx), pulses_of(h.idx));
        assert_eq!(btns.len(), pulses.len());
        // 不用真编译一遍也得数对
        for (i, n) in pulses.iter().enumerate() {
            assert_eq!(*n, code_of(h.idx, i).unwrap().sequences[0].len());
        }
    }

    #[test]
    fn 空搜索不返回全量() {
        assert!(search("", 10).is_empty());
        assert!(search("   ", 10).is_empty());
    }
}
