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
    // ✅ 使用 Vec 支持多个 MCP 路径
    #[serde(default = "default_mcp_exes")]
    pub mcp_exes: Vec<String>,
    #[serde(default)]
    pub system_prompt: String,
    #[serde(default)]
    pub embedding_api_key: String,
    #[serde(default = "default_embedding_url")]
    pub embedding_url: String,
}

fn default_url() -> String {
    "https://api.deepseek.com/chat/completions".into()
}

fn default_model() -> String {
    "deepseek-chat".into()
}

fn default_mcp_exes() -> Vec<String> {
    // 使用 r"" 原始字符串，反斜杠不需要转义
    vec![r"C:\Users\zlian\.dbx\bin\dbx-mcp.exe".to_string()]
}

fn default_embedding_url() -> String {
    "https://open.bigmodel.cn/api/paas/v4/embeddings".to_string()
}

impl Default for Config {
    fn default() -> Self {
        Self {
            deepseek_api_key: "".into(),
            deepseek_url: default_url(),
            model: default_model(),
            mcp_exes: default_mcp_exes(),
            embedding_api_key:"".into(),
            embedding_url:default_embedding_url(),
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

        // 1. 如果安全目录（AppData）里没有文件
        if !safe_path.exists() {
            // 尝试读取项目根目录下的旧 config.json
            let legacy_path = PathBuf::from("config.json");
            if legacy_path.exists() {
                if let Ok(content) = std::fs::read_to_string(&legacy_path) {
                    // 注意：旧版的 JSON 里是 "dbx_exe"，新版是 "mcp_exes"。
                    // 为了兼容，这里先按新格式反序列化，如果失败或者缺失，会触发 apply_fallbacks
                    if let Ok(cfg) = serde_json::from_str::<Config>(&content) {
                        let _ = cfg.save(); // 迁移到 AppData
                        return Ok(Self::apply_fallbacks(cfg));
                    } else {
                        // 如果解析失败（大概率因为旧版字段不匹配），直接返回默认配置并保存
                        let default_cfg = Self::default();
                        let _ = default_cfg.save();
                        return Ok(default_cfg);
                    }
                }
            }
            // 如果连旧文件都没有，说明是纯第一次运行
            let default_cfg = Self::default();
            let _ = default_cfg.save();
            return Ok(default_cfg);
        }

        // 2. 如果安全目录已有文件，正常读取
        let content = std::fs::read_to_string(&safe_path)
            .map_err(|e| anyhow::anyhow!("读取 {:?} 失败：{}", safe_path, e))?;

        // 如果解析失败，直接回退到默认配置，防止崩溃
        let cfg: Config = serde_json::from_str(&content).unwrap_or_else(|_| Self::default());

        Ok(Self::apply_fallbacks(cfg))
    }

    /// 智能兜底：如果读取出来的字段是空的，自动填入默认值
    fn apply_fallbacks(mut cfg: Config) -> Self {
        let defaults = Self::default();
        let mut changed = false;

        if cfg.deepseek_url.trim().is_empty() {
            cfg.deepseek_url = defaults.deepseek_url;
            changed = true;
        }
        if cfg.model.trim().is_empty() {
            cfg.model = defaults.model;
            changed = true;
        }

        // ✅ 检查 mcp_exes 是否为空
        if cfg.mcp_exes.is_empty() {
            cfg.mcp_exes = defaults.mcp_exes.clone();
            changed = true;
        }

        if cfg.embedding_url.trim().is_empty() {
            cfg.embedding_url = defaults.embedding_url;
            changed = true;
        }
        // embedding_api_key 允许为空（不开启 RAG），所以不用兜底

        if cfg.system_prompt.trim().is_empty() {
            cfg.system_prompt = defaults.system_prompt;
            changed = true;
        }

        if changed {
            let _ = cfg.save(); // 补全默认值后自动保存一次
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
