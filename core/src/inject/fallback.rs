use super::Injector;
use anyhow::{anyhow, Result};

pub struct NopInjector;

pub fn new_injector() -> Box<dyn Injector> {
    Box::new(NopInjector)
}
pub fn key_names() -> Vec<&'static str> {
    Vec::new()
}

impl Injector for NopInjector {
    fn available(&self) -> bool {
        false
    }
    fn why(&self) -> String {
        "本平台暂不支持按键注入".into()
    }
    fn key_down(&self, _key: &str, _mods: &[String]) -> Result<()> {
        Err(anyhow::anyhow!("这个平台没有按键注入"))
    }
    fn key_up(&self, _key: &str, _mods: &[String]) -> Result<()> {
        Err(anyhow::anyhow!("这个平台没有按键注入"))
    }
    fn key_stroke(&self, _: &str, _: &[String]) -> Result<()> {
        Err(anyhow!("不支持"))
    }
    fn type_text(&self, _: &str) -> Result<()> {
        Err(anyhow!("不支持"))
    }
}

/// 这个平台没有键码表
pub fn name_of_code(_code: u16) -> Option<&'static str> {
    None
}
