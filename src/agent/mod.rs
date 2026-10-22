pub mod loop_runner;

use crate::config::Config;

pub enum AgentCommand {
    SendMessage(String),
    ReloadConfig(Config),
    ClearHistory, // ✅ 新增
    Shutdown,
}

pub enum AgentEvent {
    Status(String),
    Ready { tool_count: usize },
    Thinking,
    AssistantStart,
    AssistantDelta(String),
    AssistantReply(String),
    ToolCallStart { name: String, args: String },
    ToolCallEnd { name: String, result: String },
    Cancelled,
    Error(String),
}