use serde_json::{json, Value};

pub fn get_local_tools() -> Vec<Value> {
    vec![
        json!({
            "type": "function",
            "function": {
                "name": "get_current_time",
                "description": "获取本地电脑当前的日期和时间。",
                "parameters": { "type": "object", "properties": {} }
            }
        }),
        json!({
            "type": "function",
            "function": {
                "name": "execute_local_shell",
                "description": "在本地电脑（Windows）执行原生 PowerShell 命令。",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "command": { "type": "string", "description": "要执行的命令" }
                    },
                    "required": ["command"]
                }
            }
        })
    ]
}

/// 执行本地工具，返回 (结果文本, 是否执行成功)
pub fn execute_local_tool(name: &str, args: &Value) -> (String, bool) {
    match name {
        "get_current_time" => {
            let now = chrono::Local::now();
            (format!("当前时间是: {}", now.format("%Y-%m-%d %H:%M:%S")), false)
        }
        "execute_local_shell" => {
            let cmd = args.get("command").and_then(|v| v.as_str()).unwrap_or("");
            if cmd.is_empty() {
                return ("缺少 command 参数".into(), true);
            }

            // 在 Windows 下调用 PowerShell 执行命令
            #[cfg(target_os = "windows")]
            let output = std::process::Command::new("powershell")
                .arg("-NoProfile")
                .arg("-Command")
                .arg(cmd)
                .output();

            #[cfg(not(target_os = "windows"))]
            let output = std::process::Command::new("sh")
                .arg("-c")
                .arg(cmd)
                .output();

            match output {
                Ok(out) => {
                    let stdout = String::from_utf8_lossy(&out.stdout);
                    let stderr = String::from_utf8_lossy(&out.stderr);
                    if out.status.success() {
                        (format!("执行成功:\n{}", stdout), false)
                    } else {
                        (format!("执行报错:\n{}", stderr), true)
                    }
                }
                Err(e) => (format!("无法启动终端: {}", e), true),
            }
        }
        _ => ("未知的本地工具".into(), true),
    }
}