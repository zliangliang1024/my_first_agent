use rmcp::{ServiceExt, transport::TokioChildProcess};
use tokio::process::Command;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let exe = r"C:\Users\zlian\.dbx\bin\dbx-mcp.exe";

    let client = ()
        .serve(TokioChildProcess::new(Command::new(exe))?)
        .await?;

    // ✅ 传入 None 表示不分页，获取全部工具
    let tools = client.list_tools(None).await?;

    println!("DBX 提供 {} 个工具：", tools.tools.len());
    for t in &tools.tools {
        println!("- {}: {}", t.name, t.description.as_deref().unwrap_or(""));
    }

    Ok(())
}