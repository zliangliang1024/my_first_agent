use rmcp::model::Tool;

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum RiskLevel {
    Low,
    High,
    Deny,
}

/// 核心策略：结合 MCP 原生注解 + 本地名称兜底
pub fn evaluate_tool_risk(tool: &Tool) -> RiskLevel {
    // ==========================================
    // 第一道防线：信任 MCP 原生注解
    // ==========================================
    if let Some(annotations) = &tool.annotations {
        // 明确声明为只读操作，直接放行
        if annotations.read_only_hint == Some(true) {
            return RiskLevel::Low;
        }
        // 明确声明为破坏性操作，必须审批
        if annotations.destructive_hint == Some(true) {
            return RiskLevel::High;
        }
    }

    // ==========================================
    // 第二道防线：如果注解缺失或不明确，走本地名称匹配
    // ==========================================
    let name = tool.name.to_lowercase();

    // 绝对红线（直接封杀）
    if name.contains("drop_table") || name.contains("format_disk") {
        return RiskLevel::Deny;
    }

    // 危险操作
    if name.contains("remove")
        || name.contains("delete")
        || name.contains("write")
        || name.contains("upload")
        || name.contains("exec")
        || name.contains("sudo")
        || name.contains("restart")
        || name.contains("kill")
    {
        return RiskLevel::High;
    }

    // 安全只读操作白名单（有些工具没写注解，但名字一看就安全）
    if name.starts_with("websearch__")
        || name.contains("_list")
        || name.contains("_stat")
        || name.contains("_get")
        || name.contains("_query")
        || name.contains("_show")
    {
        return RiskLevel::Low;
    }

    // ==========================================
    // 第三道防线：未知工具，默认高危，强制审批
    // ==========================================
    RiskLevel::High
}