pub mod loop_runner;
pub mod policy;
pub mod skill_loader; // ✅ 新增
use crate::config::Config;

pub enum AgentCommand {
    SendMessage(String),
    ReloadConfig(Config),
    ClearHistory,       // ✅ 新增
    ToolApproval(bool), // ✅ 新增：允许或拒绝工具执行
    Shutdown,
}

pub enum AgentEvent {
    Status(String),
    Ready {
        tool_count: usize,
    },
    Thinking,
    AssistantStart,
    AssistantDelta(String),
    AssistantReply(String),
    // ✅ 把 ToolCallStart 拆开，新增一个 Pending 状态
    ToolCallPending {
        id: String,
        name: String,
        args: String,
    }, // ✅ 加 id
    ToolCallStart {
        id: String,
        name: String,
        args: String,
    }, // ✅ 加 id
    ToolCallEnd {
        id: String,
        name: String,
        result: String,
    }, // ✅ 加 id
    Cancelled,
    TokenUsage {
        total_tokens: u64,
        prompt_tokens: u64,
    }, // ✅ 新增
    Error(String),
}
