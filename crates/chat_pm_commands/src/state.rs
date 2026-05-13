type DeepseekClient = chat_pm_deepseek::Client;

#[derive(Debug)]
pub struct State {
    pub deepseek_client: DeepseekClient,
}
