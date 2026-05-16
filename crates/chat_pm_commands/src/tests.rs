use std::{io::Write, time::Duration};

use anyhow::Result;
use chat_pm_session::message::UserInput;
use logforth::{
    layout::TextLayout,
    record::{Level, LevelFilter},
};

use crate::session::{ChatPipeline, PipelineConfig};
use chat_pm_database::MemoryDb;

#[tokio::test]
async fn demo() -> Result<()> {
    logforth::starter_log::stdout()
        .filter(LevelFilter::MoreSevereEqual(Level::Info))
        .layout(TextLayout::default())
        .apply();
    dotenvy::dotenv()?;

    let db = MemoryDb::open_in_memory()?;
    let config = PipelineConfig::default();
    let pipeline = ChatPipeline::with_default_deepseek(db, config)?;

    // ── 1. 创建会话，获取凭证 ───────────────────────────────────────
    let handle = pipeline.create_session();

    println!("会话已创建，session_id = {}", handle.id());

    // ── 2. 模拟多轮对话 ─────────────────────────────────────────────
    let turns = [
        "你好",
        "帮我规划日本旅行，我喜欢二次元和乡村，先推荐第一天。",
        "帮我规划日本旅行，我喜欢二次元和乡村，再推荐第二天。",
    ];

    for (i, user_input) in turns.iter().enumerate() {
        println!("{}", "─".repeat(60));
        println!("轮次 {} 用户：{}", i + 1, user_input);
        println!("{}", "─".repeat(60));

        let mut stream = pipeline.chat(&handle, UserInput::new(user_input)).await?;
        print!("助手：");
        while let Some(Ok(frame)) = stream.recv().await {
            print(&frame.content);
            // if let Some(warn) = &answer.truncation_warning {
            //     println!("⚠️  {warn}");
            // }
        }

        println!();
    }

    // ── 3. 模拟跨请求恢复（HTTP 无状态场景） ────────────────────────
    println!("{}", "═".repeat(60));
    println!("(模拟：新 HTTP 请求携带旧 session_id 恢复会话)");
    println!("用户：刚才我们聊到哪里了？");
    println!("{}", "═".repeat(60));

    let saved_id = handle.id(); // 前端 cookie 中保存的值

    match pipeline.resume_session(saved_id) {
        Ok(resumed) => {
            let mut stream = pipeline
                .chat(&resumed, UserInput::new("刚才我们聊到哪里了？"))
                .await?;
            print!("助手：");
            while let Some(Ok(frame)) = stream.recv().await {
                print!("{}", frame.content);
                // if let Some(warn) = &answer.truncation_warning {
                //     println!("⚠️  {warn}");
                // }
            }
        }
        Err(e) => eprintln!("恢复失败：{e}"),
    }

    Ok(())
}

fn print(s: &str) {
    for c in s.chars() {
        print!("{}", c);
        std::io::stdout().flush().unwrap();
        std::thread::sleep(Duration::from_millis(10));
    }
}
