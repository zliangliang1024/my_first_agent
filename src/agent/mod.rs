pub mod loop_runner;

use crate::config::Config;

pub enum AgentCommand {
    SendMessage(String),
    ReloadConfig(Config),
    ClearHistory, // ✅ 新增
    ToolApproval(bool), // ✅ 新增：允许或拒绝工具执行
    Shutdown,
}

pub enum AgentEvent {
    Status(String),
    Ready { tool_count: usize },
    Thinking,
    AssistantStart,
    AssistantDelta(String),
    AssistantReply(String),
     // ✅ 把 ToolCallStart 拆开，新增一个 Pending 状态
    ToolCallPending { name: String, args: String },
    ToolCallStart { name: String, args: String },
    ToolCallEnd { name: String, result: String },
    Cancelled,
    Error(String),
}