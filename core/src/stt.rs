//! 自带语音识别：系统的 `SFSpeechRecognizer`，离线、中文原生、不用下模型。
//!
//! **走「文件识别」而不是实时 buffer**：按住期间把 PCM 攒在内存，松手写一个
//! 临时 WAV 再识别。这样不用构造 `AVAudioPCMBuffer`（少一大坨 unsafe），
//! 而且能用 `say` 合成语音自测，不依赖遥控器。代价是没有实时中间结果 ——
//! 对「按住说话、松手出字」这个用法无所谓。
//!
//! 需要「语音识别」权限（Info.plist 里的 `NSSpeechRecognitionUsageDescription`），
//! 首次用会弹系统授权框。

use anyhow::{anyhow, Result};
use std::path::Path;

/// 系统语音识别实际支持的语言。名称预先按中英文界面各生成一份，UI 切换语言时
/// 不需要重新查询 Speech.framework。
#[derive(Clone, Debug)]
pub struct SpeechLocale {
    pub identifier: String,
    pub zh_name: String,
    pub en_name: String,
}

/// 攒 PCM 的缓冲。16 kHz 单声道 i16，和遥控器解码出来的一致。
#[derive(Default)]
pub struct Recorder {
    pcm: Vec<i16>,
    /// 最近一小段的峰值（0~1）。听写不过虚拟声卡，
    /// 所以 VoiceSink 那边的电平是 0，悬浮条只能读这个。
    peak: f32,
    /// 按下麦克风那一刻谁在前台 —— 识别完打字要打回它身上
    pub front: Option<crate::frontapp::FrontApp>,
}

impl Recorder {
    pub fn new() -> Self {
        Self::default()
    }
    pub fn clear(&mut self) {
        self.pcm.clear();
    }
    pub fn push(&mut self, s: &[i16]) {
        // 上限 60 秒，别无限吃内存
        if self.pcm.len() < 16_000 * 60 {
            self.pcm.extend_from_slice(s);
        }
        let pk = s
            .iter()
            .map(|v| (*v as f32 / 32768.0).abs())
            .fold(0.0_f32, f32::max);
        // 上升立刻跟，下降慢一点，不然条子闪得看不清
        self.peak = if pk > self.peak {
            pk
        } else {
            self.peak * 0.82 + pk * 0.18
        };
    }
    /// 实时电平 0~1
    pub fn level(&self) -> f32 {
        self.peak
    }
    pub fn samples(&self) -> usize {
        self.pcm.len()
    }
    pub fn seconds(&self) -> f32 {
        self.pcm.len() as f32 / 16_000.0
    }
    /// 写成 WAV 临时文件，返回路径
    pub fn write_wav(&self, rate: u32) -> Result<std::path::PathBuf> {
        let dir = std::env::temp_dir();
        // 带序号：连着两次听写不能共用一个文件名，
        // 否则先完成那次的清理会把后一次的文件删掉
        static N: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
        let n = N.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let path = dir.join(format!("firevibe-stt-{}-{n}.wav", std::process::id()));
        std::fs::write(&path, wav_bytes(&self.pcm, rate))?;
        Ok(path)
    }
}

/// 最小 WAV 封装：16-bit PCM 单声道
fn wav_bytes(pcm: &[i16], rate: u32) -> Vec<u8> {
    let data_len = (pcm.len() * 2) as u32;
    let mut v = Vec::with_capacity(44 + data_len as usize);
    v.extend_from_slice(b"RIFF");
    v.extend_from_slice(&(36 + data_len).to_le_bytes());
    v.extend_from_slice(b"WAVEfmt ");
    v.extend_from_slice(&16u32.to_le_bytes()); // fmt chunk 大小
    v.extend_from_slice(&1u16.to_le_bytes()); // PCM
    v.extend_from_slice(&1u16.to_le_bytes()); // 单声道
    v.extend_from_slice(&rate.to_le_bytes());
    v.extend_from_slice(&(rate * 2).to_le_bytes()); // 字节率
    v.extend_from_slice(&2u16.to_le_bytes()); // 块对齐
    v.extend_from_slice(&16u16.to_le_bytes()); // 位深
    v.extend_from_slice(b"data");
    v.extend_from_slice(&data_len.to_le_bytes());
    for s in pcm {
        v.extend_from_slice(&s.to_le_bytes());
    }
    v
}

#[cfg(target_os = "macos")]
mod imp {
    use super::*;
    use block2::RcBlock;
    use objc2::rc::Retained;
    use objc2::AnyThread;
    use objc2_foundation::{NSLocale, NSString, NSURL};
    use objc2_speech::{
        SFSpeechRecognitionResult, SFSpeechRecognizer, SFSpeechRecognizerAuthorizationStatus,
        SFSpeechURLRecognitionRequest,
    };
    use std::sync::mpsc;
    use std::time::Duration;

    /// 当前授权状态
    pub fn auth_status() -> &'static str {
        match unsafe { SFSpeechRecognizer::authorizationStatus() } {
            SFSpeechRecognizerAuthorizationStatus::Authorized => "已授权",
            SFSpeechRecognizerAuthorizationStatus::Denied => "被拒绝",
            SFSpeechRecognizerAuthorizationStatus::Restricted => "受限",
            _ => "未决定",
        }
    }

    pub fn authorized() -> bool {
        let st = unsafe { SFSpeechRecognizer::authorizationStatus() };
        st == SFSpeechRecognizerAuthorizationStatus::Authorized
    }

    /// 直接取 Speech.framework 在当前系统上公布的语言列表，而不是在界面里写死
    /// 中英文两个选项。不同 macOS 版本、地区和已安装听写资源会得到不同集合。
    pub fn supported_locales() -> Vec<SpeechLocale> {
        unsafe {
            let zh = NSLocale::initWithLocaleIdentifier(
                NSLocale::alloc(),
                &NSString::from_str("zh-CN"),
            );
            let en = NSLocale::initWithLocaleIdentifier(
                NSLocale::alloc(),
                &NSString::from_str("en-US"),
            );
            let mut locales = SFSpeechRecognizer::supportedLocales()
                .iter()
                .map(|locale| {
                    let identifier = locale.localeIdentifier();
                    let identifier_text = identifier.to_string();
                    let zh_name = zh
                        .localizedStringForLocaleIdentifier(&identifier)
                        .to_string();
                    let en_name = en
                        .localizedStringForLocaleIdentifier(&identifier)
                        .to_string();
                    SpeechLocale {
                        identifier: identifier_text.clone(),
                        zh_name: if zh_name.trim().is_empty() {
                            identifier_text.clone()
                        } else {
                            zh_name
                        },
                        en_name: if en_name.trim().is_empty() {
                            identifier_text
                        } else {
                            en_name
                        },
                    }
                })
                .collect::<Vec<_>>();
            locales.sort_by(|a, b| a.en_name.cmp(&b.en_name));
            locales
        }
    }

    /// 请求授权（首次会弹系统框）。等最多 60 秒。
    pub fn request_auth() -> Result<bool> {
        if authorized() {
            return Ok(true);
        }
        let (tx, rx) = mpsc::channel();
        let handler = RcBlock::new(move |st: SFSpeechRecognizerAuthorizationStatus| {
            let _ = tx.send(st == SFSpeechRecognizerAuthorizationStatus::Authorized);
        });
        unsafe { SFSpeechRecognizer::requestAuthorization(&handler) };
        rx.recv_timeout(Duration::from_secs(60))
            .map_err(|_| anyhow!("等授权超时"))
    }

    fn recognizer(locale: &str) -> Result<Retained<SFSpeechRecognizer>> {
        unsafe {
            let loc =
                NSLocale::initWithLocaleIdentifier(NSLocale::alloc(), &NSString::from_str(locale));
            SFSpeechRecognizer::initWithLocale(SFSpeechRecognizer::alloc(), &loc)
                .ok_or_else(|| anyhow!("这个语言不支持语音识别: {locale}"))
        }
    }

    /// 识别一个音频文件。`on_device` 为真时强制离线（不联网）。
    pub fn transcribe_file(path: &Path, locale: &str, on_device: bool) -> Result<String> {
        if !authorized() {
            return Err(anyhow!("没有语音识别权限（当前: {}）", auth_status()));
        }
        let rec = recognizer(locale)?;
        if unsafe { !rec.isAvailable() } {
            return Err(anyhow!("语音识别暂时不可用"));
        }
        let (tx, rx) = mpsc::channel::<Result<String, String>>();
        unsafe {
            let url = NSURL::fileURLWithPath(&NSString::from_str(
                path.to_str().ok_or_else(|| anyhow!("路径不是 UTF-8"))?,
            ));
            let req = SFSpeechURLRecognitionRequest::initWithURL(
                SFSpeechURLRecognitionRequest::alloc(),
                &url,
            );
            req.setShouldReportPartialResults(false);
            req.setAddsPunctuation(true);
            if on_device && rec.supportsOnDeviceRecognition() {
                req.setRequiresOnDeviceRecognition(true);
            }
            let handler = RcBlock::new(
                move |res: *mut SFSpeechRecognitionResult, err: *mut objc2_foundation::NSError| {
                    if !err.is_null() {
                        let e = &*err;
                        let _ = tx.send(Err(e.localizedDescription().to_string()));
                        return;
                    }
                    if res.is_null() {
                        return;
                    }
                    let r = &*res;
                    if r.isFinal() {
                        let _ = tx.send(Ok(r.bestTranscription().formattedString().to_string()));
                    }
                },
            );
            let _task = rec.recognitionTaskWithRequest_resultHandler(&req, &handler);
            // task 要活到回调结束；这里同步等结果，_task 一直在栈上
            match rx.recv_timeout(Duration::from_secs(30)) {
                Ok(Ok(s)) => Ok(s),
                Ok(Err(e)) => Err(anyhow!("识别失败: {e}")),
                Err(_) => Err(anyhow!("识别超时")),
            }
        }
    }
}

#[cfg(not(target_os = "macos"))]
mod imp {
    use super::*;
    pub fn auth_status() -> &'static str {
        "这个平台没有自带语音识别"
    }
    pub fn authorized() -> bool {
        false
    }
    pub fn request_auth() -> Result<bool> {
        Ok(false)
    }
    pub fn supported_locales() -> Vec<SpeechLocale> {
        vec![
            SpeechLocale {
                identifier: "zh-CN".into(),
                zh_name: "简体中文（中国大陆）".into(),
                en_name: "Chinese (China mainland)".into(),
            },
            SpeechLocale {
                identifier: "en-US".into(),
                zh_name: "英语（美国）".into(),
                en_name: "English (United States)".into(),
            },
        ]
    }
    pub fn transcribe_file(_p: &Path, _l: &str, _d: bool) -> Result<String> {
        Err(anyhow!("这个平台没有自带语音识别"))
    }
}

pub use imp::{auth_status, authorized, request_auth, supported_locales, transcribe_file};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wav_header_is_sane() {
        let b = wav_bytes(&[0i16, 1, -1, 32767], 16_000);
        assert_eq!(&b[0..4], b"RIFF");
        assert_eq!(&b[8..12], b"WAVE");
        assert_eq!(&b[36..40], b"data");
        assert_eq!(b.len(), 44 + 8);
        // 采样率字段
        assert_eq!(u32::from_le_bytes(b[24..28].try_into().unwrap()), 16_000);
    }

    /// 端到端自测，不用遥控器：用 `say` 合成一段语音再识别。
    /// 手动跑：`cargo test -p firevibe-core -- --ignored --nocapture stt_roundtrip`
    #[test]
    #[ignore = "要语音识别权限，首次会弹授权框"]
    fn stt_roundtrip_with_say() {
        println!("授权状态: {}", auth_status());
        if !authorized() {
            println!("请求授权…");
            let ok = request_auth().unwrap_or(false);
            println!("授权结果: {ok}（{}）", auth_status());
            assert!(ok, "没拿到语音识别权限");
        }
        let wav = std::env::temp_dir().join("firevibe-say-test.wav");
        let phrase = "今天天气很好";
        let st = std::process::Command::new("say")
            .args(["-v", "Tingting", "--data-format=LEI16@16000", "-o"])
            .arg(&wav)
            .arg(phrase)
            .status()
            .expect("say");
        assert!(st.success(), "say 没跑成");
        let got = transcribe_file(&wav, "zh-CN", true).expect("识别");
        println!("说的: {phrase}\n识别: {got}");
        assert!(!got.trim().is_empty(), "识别结果是空的");
    }
}
