//! 录音到「下载」。按一下开始、再按一下停止。
//!
//! **直接吃遥控器解码后的 PCM** —— 不开系统麦克风、不碰默认输入设备。
//! HID 线程本来就在解码音频（听写走的是同一路），这里只是多接一路写文件。
//!
//! **不驱动 QuickTime**：需求里有「录音状态只在 app 窗口里显示」，
//! QuickTime 会弹自己的窗口和保存对话框，直接违背这条。
//!
//! WAV 是**流式写**的：先占位写头，边录边追加，停止时回填长度 ——
//! 录一小时也不吃内存。

use anyhow::{Context, Result};
use std::fs::File;
use std::io::{Seek, SeekFrom, Write};
use std::path::PathBuf;
use std::time::{Instant, SystemTime, UNIX_EPOCH};

pub struct Rec {
    file: Option<File>,
    path: PathBuf,
    rate: u32,
    samples: u64,
    peak: f32,
    started: Instant,
}

impl Rec {
    /// 建文件、写占位头。`rate` 用遥控器的采样率（16k）。
    pub fn start(rate: u32) -> Result<Self> {
        let path = out_path(&stamp_now());
        let mut f =
            File::create(&path).with_context(|| format!("建文件失败 {}", path.display()))?;
        f.write_all(&wav_header(rate))?;
        Ok(Self {
            file: Some(f),
            path,
            rate,
            samples: 0,
            peak: 0.0,
            started: Instant::now(),
        })
    }

    pub fn push(&mut self, pcm: &[i16]) {
        let pk = pcm
            .iter()
            .map(|v| (*v as f32 / 32768.0).abs())
            .fold(0.0_f32, f32::max);
        // 上升立刻跟，下降慢一点，界面上的电平才看得清
        self.peak = if pk > self.peak {
            pk
        } else {
            self.peak * 0.82 + pk * 0.18
        };
        if let Some(f) = self.file.as_mut() {
            let mut buf = Vec::with_capacity(pcm.len() * 2);
            for s in pcm {
                buf.extend_from_slice(&s.to_le_bytes());
            }
            if f.write_all(&buf).is_ok() {
                self.samples += pcm.len() as u64;
            }
        }
    }

    /// 已写入的采样数（日志限频用）
    pub fn samples_len(&self) -> u64 {
        self.samples
    }
    pub fn level(&self) -> f32 {
        self.peak
    }
    /// 已录时长（按实际写入的采样数算，比挂钟准）
    pub fn seconds(&self) -> f32 {
        self.samples as f32 / self.rate.max(1) as f32
    }
    /// 挂钟时长 —— 界面上的计时器用这个，即使音频还没来也在走
    pub fn elapsed(&self) -> f32 {
        self.started.elapsed().as_secs_f32()
    }
    pub fn path(&self) -> &std::path::Path {
        &self.path
    }

    /// 停止：回填 WAV 头的长度字段。返回路径和时长。
    pub fn finish(mut self) -> Result<(PathBuf, f32)> {
        let n = self.samples;
        if let Some(mut f) = self.file.take() {
            let data_len = (n * 2) as u32;
            f.seek(SeekFrom::Start(4))?;
            f.write_all(&(36 + data_len).to_le_bytes())?;
            f.seek(SeekFrom::Start(40))?;
            f.write_all(&data_len.to_le_bytes())?;
            f.flush()?;
        }
        Ok((self.path, n as f32 / self.rate.max(1) as f32))
    }
}

/// 「下载」目录 + 带时间戳的文件名
fn out_path(stamp: &str) -> PathBuf {
    dirs::home_dir()
        .map(|h| h.join("Downloads"))
        .unwrap_or_else(|| PathBuf::from("."))
        .join(format!("FireVibe 录音 {stamp}.wav"))
}

/// `2026-08-23 01-23-45`（本地时区）。自己算，不引日期库。
fn stamp_now() -> String {
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    let off = local_offset_secs();
    fmt_stamp(secs + off)
}

/// 本地时区相对 UTC 的偏移（秒）。用 libc 的 localtime 拿，省得自己查 tz 库。
fn local_offset_secs() -> i64 {
    unsafe {
        let t: libc::time_t = 0;
        let mut tm: libc::tm = std::mem::zeroed();
        libc::localtime_r(&t, &mut tm);
        tm.tm_gmtoff as i64
    }
}

/// Unix 秒 → `YYYY-MM-DD HH-MM-SS`（已含时区偏移的「本地秒」）
fn fmt_stamp(local_secs: i64) -> String {
    let days = local_secs.div_euclid(86_400);
    let rem = local_secs.rem_euclid(86_400);
    let (h, mi, s) = (rem / 3600, (rem % 3600) / 60, rem % 60);
    let (y, mo, d) = civil_from_days(days);
    format!("{y:04}-{mo:02}-{d:02} {h:02}-{mi:02}-{s:02}")
}

/// Howard Hinnant 那套 days→civil。1970-01-01 = 0。
fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (if m <= 2 { y + 1 } else { y }, m, d)
}

/// 16-bit PCM 单声道 WAV 头，长度先占位（停止时回填）
fn wav_header(rate: u32) -> [u8; 44] {
    let mut h = [0u8; 44];
    h[..4].copy_from_slice(b"RIFF");
    h[8..12].copy_from_slice(b"WAVE");
    h[12..16].copy_from_slice(b"fmt ");
    h[16..20].copy_from_slice(&16u32.to_le_bytes());
    h[20..22].copy_from_slice(&1u16.to_le_bytes()); // PCM
    h[22..24].copy_from_slice(&1u16.to_le_bytes()); // 单声道
    h[24..28].copy_from_slice(&rate.to_le_bytes());
    h[28..32].copy_from_slice(&(rate * 2).to_le_bytes());
    h[32..34].copy_from_slice(&2u16.to_le_bytes());
    h[34..36].copy_from_slice(&16u16.to_le_bytes());
    h[36..40].copy_from_slice(b"data");
    h
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn header_is_a_valid_wav_skeleton() {
        let h = wav_header(16_000);
        assert_eq!(&h[..4], b"RIFF");
        assert_eq!(&h[8..12], b"WAVE");
        assert_eq!(&h[36..40], b"data");
        assert_eq!(u16::from_le_bytes([h[22], h[23]]), 1, "单声道");
        assert_eq!(u16::from_le_bytes([h[34], h[35]]), 16, "16 bit");
        assert_eq!(
            u32::from_le_bytes([h[28], h[29], h[30], h[31]]),
            32_000,
            "字节率 = 采样率 × 2"
        );
    }

    #[test]
    fn output_goes_to_downloads_as_wav() {
        let p = out_path("2026-08-23 01-23-45");
        assert!(p.to_string_lossy().contains("Downloads"), "{p:?}");
        assert!(p.extension().is_some_and(|e| e == "wav"));
        assert!(p.to_string_lossy().contains("2026-08-23 01-23-45"));
    }

    #[test]
    fn timestamp_math_is_right() {
        // 1970-01-01 00:00:00
        assert_eq!(fmt_stamp(0), "1970-01-01 00-00-00");
        // 2026-08-23 01:23:45 UTC = 1787448225（用 python 独立核对过）
        assert_eq!(fmt_stamp(1_787_448_225), "2026-08-23 01-23-45");
        // 闰年边界
        assert_eq!(fmt_stamp(1_709_164_800), "2024-02-29 00-00-00");
    }

    #[test]
    fn writes_and_finalizes() {
        let mut r = Rec::start(16_000).expect("建文件");
        let p = r.path().to_path_buf();
        r.push(&[0i16; 1600]); // 0.1 秒
        r.push(&[16_000i16; 1600]);
        assert!((r.seconds() - 0.2).abs() < 0.01, "时长 {}", r.seconds());
        assert!(r.level() > 0.3, "电平应该跟着涨: {}", r.level());
        let (path, secs) = r.finish().expect("收尾");
        let meta = std::fs::metadata(&path).expect("文件在");
        assert_eq!(meta.len(), 44 + 3200 * 2, "头 44 + 3200 采样 × 2 字节");
        assert!((secs - 0.2).abs() < 0.01);
        // 头里的长度回填对了吗
        let raw = std::fs::read(&path).unwrap();
        let data_len = u32::from_le_bytes([raw[40], raw[41], raw[42], raw[43]]);
        assert_eq!(data_len, 3200 * 2);
        let _ = std::fs::remove_file(&p);
    }
}
