use anyhow::Result;
use logforth::{
    layout::TextLayout,
    record::{Level, LevelFilter},
};

use crate::session::{ChatPipeline, PipelineConfig};
use chat_pm_database::MemoryDb;

#[tokio::test]
async fn main() -> Result<()> {
    logforth::starter_log::stdout()
        .filter(LevelFilter::MoreSevereEqual(Level::Debug))
        .layout(TextLayout::default())
        .apply();
    dotenvy::dotenv()?;

    let db = MemoryDb::new();
    let config = PipelineConfig::default();
    let pipeline = ChatPipeline::new(db, config);

    // ── 1. 创建会话，获取凭证 ───────────────────────────────────────
    // create_session 是 session_id 的唯一来源，返回 SessionHandle。
    // handle.id() 暴露底层字符串，供 HTTP 响应头 / cookie 返回给前端。
    let handle = pipeline.create_session();

    println!("会话已创建，session_id = {}", handle.id());
    println!("（前端应保存此 ID，下次通过 resume_session 恢复）\n");

    // ── 2. 模拟多轮对话 ─────────────────────────────────────────────
    // 始终传 &handle，不需要手动传递 session_id 字符串
    let turns = [
        "你好",
        "什么是 Rust 的所有权系统？",
        "能用一个代码例子说明借用规则吗？",
        "上面那个例子里，如果我需要多个可变引用怎么办？",
    ];

    for (i, user_input) in turns.iter().enumerate() {
        println!("{}", "─".repeat(60));
        println!("轮次 {} 用户：{}", i + 1, user_input);
        println!("{}", "─".repeat(60));

        match pipeline.chat(&handle, user_input).await {
            Ok(answer) => {
                println!("助手：{}", answer.display_text);
                if let Some(warn) = &answer.truncation_warning {
                    println!("⚠️  {warn}");
                }
            }
            Err(e) => eprintln!("❌ 错误：{e:#}"),
        }

        println!();
    }

    // ── 3. 模拟跨请求恢复（HTTP 无状态场景） ────────────────────────
    println!("{}", "═".repeat(60));
    println!("模拟：新 HTTP 请求携带旧 session_id 恢复会话");

    let saved_id = handle.id(); // 前端 cookie 中保存的值

    match pipeline.resume_session(saved_id) {
        Ok(resumed) => {
            let answer = pipeline.chat(&resumed, "刚才我们聊到哪里了？").await?;
            println!("恢复后回复：{}", answer.display_text);
        }
        Err(e) => eprintln!("恢复失败：{e}"),
    }

    Ok(())
}
