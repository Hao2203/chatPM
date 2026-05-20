use std::{io::Write, time::Duration};

use anyhow::Result;
use chat_pm_session::message::UserInput;
use logforth::{
    layout::TextLayout,
    record::{Level, LevelFilter},
};

use crate::session::{ChatService, ChatConfig};
use chat_pm_database::ChatDb;

#[tokio::test]
async fn demo() -> Result<()> {
    logforth::starter_log::stdout()
        .filter(LevelFilter::MoreSevereEqual(Level::Info))
        .layout(TextLayout::default())
        .apply();
    dotenvy::dotenv()?;

    let db = ChatDb::open_in_memory()?;
    let config = ChatConfig::default();
    let service = ChatService::with_default_deepseek(db, config)?;

    // ── 1. 创建会话 → NewSession ──────────────────────────────────
    let mut new_session = Some(service.create_session()?);
    println!(
        "会话已创建，session_id = {}",
        new_session.as_ref().unwrap().session_id()
    );

    // ── 2. 首轮：NewSession → TitlePrompt → Session → chat ──────
    let turns = [
        "你好",
        "帮我规划日本旅行，我喜欢二次元和乡村，先推荐第一天。",
        "帮我规划日本旅行，我喜欢二次元和乡村，再推荐第二天。",
    ];

    let mut session: Option<chat_pm_session::session::Session> = None;

    for (i, user_text) in turns.iter().enumerate() {
        println!("{}", "─".repeat(60));
        println!("轮次 {} 用户：{}", i + 1, user_text);
        println!("{}", "─".repeat(60));

        if i == 0 {
            // 首轮：NewSession → TitlePrompt → finalize → Session
            let title_input = UserInput::new(user_text);
            let tp = new_session.take().unwrap().into_title_prompt(&title_input);
            let s = service.finalize_session(tp).await?;
            println!("标题已生成：{}", s.title());
            session = Some(s);
        }

        let s = session.as_ref().unwrap();
        let mut stream = service.chat(s, UserInput::new(user_text)).await?;
        print!("助手：");
        while let Some(Ok(frame)) = stream.recv().await {
            print(&frame.content);
        }
        println!();
    }

    // ── 3. 模拟跨请求恢复 ──────────────────────────────────────
    println!("{}", "═".repeat(60));
    println!("(模拟：新 HTTP 请求携带旧 session_id 恢复会话)");
    println!("用户：刚才我们聊到哪里了？");
    println!("{}", "═".repeat(60));

    let saved_id = session.as_ref().unwrap().session_id();

    match service.resume_session(saved_id) {
        Ok(resumed) => {
            let mut stream = service
                .chat(&resumed, UserInput::new("刚才我们聊到哪里了？"))
                .await?;
            print!("助手：");
            while let Some(Ok(frame)) = stream.recv().await {
                print!("{}", frame.content);
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
