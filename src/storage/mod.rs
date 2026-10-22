use anyhow::Result;
use serde::{de::DeserializeOwned, Serialize};
use std::path::PathBuf;

pub fn get_storage_path(filename: &str) -> PathBuf {
    let mut path = dirs::config_dir().unwrap_or_else(|| PathBuf::from("."));
    path.push("my_first_agent");
    let _ = std::fs::create_dir_all(&path);
    path.push(filename);
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