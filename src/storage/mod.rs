use serde::{Deserialize, Serialize, de::DeserializeOwned};
use std::path::PathBuf;

pub fn get_storage_path(filename: &str) -> PathBuf {
    let mut path = dirs::config_dir().unwrap_or_else(|| PathBuf::from("."));
    path.push("my_first_agent");
    let _ = std::fs::create_dir_all(&path);
    path.push(filename);
    path
}

// 专门用来存 session 的文件夹
pub fn get_sessions_dir() -> PathBuf {
    let mut path = dirs::config_dir().unwrap_or_else(|| PathBuf::from("."));
    path.push("my_first_agent");
    path.push("sessions");
    let _ = std::fs::create_dir_all(&path);
    path
}

pub fn save_json<T: Serialize + ?Sized>(filename: &str, data: &T) {
    let path = get_storage_path(filename);
    if let Ok(json) = serde_json::to_string_pretty(data) {
        let _ = std::fs::write(path, json);
    }
}

pub fn load_json<T: DeserializeOwned>(filename: &str) -> Option<T> {
    let path = get_storage_path(filename);
    if let Ok(content) = std::fs::read_to_string(path) {
        if let Ok(data) = serde_json::from_str(&content) {
            return Some(data);
        }
    }
    None
}

// ==========================================
// ✅ 新增：多会话存储逻辑
// ==========================================

#[derive(Serialize, Deserialize, Clone)]
pub struct SessionMeta {
    pub id: String,
    pub title: String,
}

pub fn save_session_ui(id: &str, chat: &[crate::ui::app::ChatItem]) {
    let path = get_sessions_dir().join(format!("{}_ui.json", id));
    if let Ok(json) = serde_json::to_string_pretty(chat) {
        let _ = std::fs::write(path, json);
    }
}

pub fn load_session_ui(id: &str) -> Vec<crate::ui::app::ChatItem> {
    let path = get_sessions_dir().join(format!("{}_ui.json", id));
    if let Ok(content) = std::fs::read_to_string(path) {
        if let Ok(data) = serde_json::from_str(&content) {
            return data;
        }
    }
    Vec::new()
}

pub fn save_session_llm(id: &str, history: &[crate::llm::ChatMessage]) {
    let path = get_sessions_dir().join(format!("{}_llm.json", id));
    if let Ok(json) = serde_json::to_string_pretty(history) {
        let _ = std::fs::write(path, json);
    }
}

pub fn load_session_llm(id: &str) -> Vec<crate::llm::ChatMessage> {
    let path = get_sessions_dir().join(format!("{}_llm.json", id));
    if let Ok(content) = std::fs::read_to_string(path) {
        if let Ok(data) = serde_json::from_str(&content) {
            return data;
        }
    }
    Vec::new()
}

// 读取所有会话的索引
pub fn load_sessions_index() -> Vec<SessionMeta> {
    let path = get_storage_path("sessions_index.json");
    if let Ok(content) = std::fs::read_to_string(path) {
        if let Ok(data) = serde_json::from_str(&content) {
            return data;
        }
    }
    Vec::new()
}

// 保存会话索引
pub fn save_sessions_index(sessions: &[SessionMeta]) {
    let path = get_storage_path("sessions_index.json");
    if let Ok(json) = serde_json::to_string_pretty(sessions) {
        let _ = std::fs::write(path, json);
    }
}

// ✅ 新增：删除会话的物理文件
pub fn delete_session_files(id: &str) {
    let ui_path = get_sessions_dir().join(format!("{}_ui.json", id));
    let llm_path = get_sessions_dir().join(format!("{}_llm.json", id));
    let _ = std::fs::remove_file(ui_path);
    let _ = std::fs::remove_file(llm_path);
}

// ✅ 新增：追加审计日志
pub fn append_audit_log(tool_name: &str, args: &str, result: &str) {
    use std::io::Write;
    let path = get_storage_path("audit.log");

    // 格式化当前时间
    let now = chrono::Local::now().format("%Y-%m-%d %H:%M:%S").to_string();

    // 为了防止日志太长，截断一下结果
    let short_result = if result.len() > 200 {
        format!("{}...", &result[..200])
    } else {
        result.to_string()
    };

    let log_line = format!(
        "[{}] Tool: {} | Args: {} | Result: {}\n",
        now, tool_name, args, short_result
    );

    if let Ok(mut file) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
    {
        let _ = file.write_all(log_line.as_bytes());
    }
}
