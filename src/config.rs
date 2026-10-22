use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Config {
    pub deepseek_api_key: String,
    #[serde(default = "default_url")]
    pub deepseek_url: String,
    #[serde(default = "default_model")]
    pub model: String,
    pub dbx_exe: String,
    #[serde(default)]
    pub system_prompt: String,
}

fn default_url() -> String { "https://api.deepseek.com/chat/completions".into() }
fn default_model() -> String { "deepseek-chat".into() }

// ✅ 实现真正的默认值
impl Default for Config {
    fn default() -> Self {
        Self {
            deepseek_api_key: "".into(),
            deepseek_url: default_url(),
            model: default_model(),
            dbx_exe: r"C:\Users\zlian\.dbx\bin\dbx-mcp.exe".into(), // 你的常用路径
            system_prompt: "你是一个专业的运维助手 Agent，可以调用 MCP 工具在本地开发机和远程 Linux 服务器上执行命令。\n\n行为准则：\n1. 优先调用工具获取真实数据，不要凭猜测回答。\n2. 涉及删除、修改、sudo 等危险操作，先向用户说明你要做什么，得到确认再执行。\n3. 回复使用中文，重要数据用表格或列表展示。".into(),
        }
    }
}

impl Config {
    pub fn get_config_path() -> PathBuf {
        if let Ok(custom_path) = std::env::var("AGENT_CONFIG") {
            return PathBuf::from(custom_path);
        }
        let mut path = dirs::config_dir().unwrap_or_else(|| PathBuf::from("."));
        path.push("my_first_agent");
        let _ = std::fs::create_dir_all(&path);
        path.push("config.json");
        path
    }

    pub fn load() -> Result<Self> {
        let safe_path = Self::get_config_path();
        if !safe_path.exists() {
            let legacy_path = PathBuf::from("config.json");
            if legacy_path.exists() {
                if let Ok(content) = std::fs::read_to_string(&legacy_path) {
                    if let Ok(cfg) = serde_json::from_str::<Config>(&content) {
                        let _ = cfg.save(); 
                        return Ok(cfg);
                    }
                }
            }
            return Ok(Self::default());
        }

        let content = std::fs::read_to_string(&safe_path)
            .map_err(|e| anyhow::anyhow!("读取 {:?} 失败：{}", safe_path, e))?;
        let cfg: Config = serde_json::from_str(&content)
            .map_err(|e| anyhow::anyhow!("解析 {:?} 失败：{}", safe_path, e))?;
        Ok(cfg)
    }

    pub fn save(&self) -> Result<()> {
        let path = Self::get_config_path();
        let content = serde_json::to_string_pretty(self)?;
        std::fs::write(&path, content)?;
        Ok(())
    }
}