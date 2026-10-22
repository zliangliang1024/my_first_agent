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

impl Default for Config {
    fn default() -> Self {
        Self {
            deepseek_api_key: "".into(),
            deepseek_url: default_url(),
            model: default_model(),
            dbx_exe: r"C:\Users\zlian\.dbx\bin\dbx-mcp.exe".into(),
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
        println!("[Config] 准备读取安全路径: {:?}", safe_path);
        
        if !safe_path.exists() {
            println!("[Config] 安全路径不存在，尝试寻找旧文件...");
            let legacy_path = PathBuf::from("config.json");
            if legacy_path.exists() {
                if let Ok(content) = std::fs::read_to_string(&legacy_path) {
                    if let Ok(cfg) = serde_json::from_str::<Config>(&content) {
                        let _ = cfg.save();
                        println!("[Config] 旧文件迁移成功！");
                        return Ok(Self::apply_fallbacks(cfg));
                    }
                }
            }
            println!("[Config] 未找到任何配置文件，使用全默认配置。");
            return Ok(Self::default());
        }

        println!("[Config] 安全路径文件已存在，直接读取...");
        let content = std::fs::read_to_string(&safe_path)
            .map_err(|e| anyhow::anyhow!("读取 {:?} 失败：{}", safe_path, e))?;
        
        let cfg: Config = serde_json::from_str(&content).unwrap_or_else(|e| {
            println!("[Config] 解析失败: {}，强制重置为默认配置。", e);
            Self::default()
        });
        
        Ok(Self::apply_fallbacks(cfg))
    }

    // ✅ 智能兜底：如果字段是空字符串，自动填入默认值
    fn apply_fallbacks(mut cfg: Config) -> Self {
        let defaults = Self::default();
        let mut changed = false;

        if cfg.deepseek_url.trim().is_empty() { cfg.deepseek_url = defaults.deepseek_url; changed = true; }
        if cfg.model.trim().is_empty() { cfg.model = defaults.model; changed = true; }
        if cfg.dbx_exe.trim().is_empty() { cfg.dbx_exe = defaults.dbx_exe; changed = true; }
        if cfg.system_prompt.trim().is_empty() { cfg.system_prompt = defaults.system_prompt; changed = true; }

        if changed {
            println!("[Config] 发现空字段，已自动补充默认配置。");
        }
        cfg
    }

    pub fn save(&self) -> Result<()> {
        let path = Self::get_config_path();
        let content = serde_json::to_string_pretty(self)?;
        std::fs::write(&path, content)?;
        Ok(())
    }
}