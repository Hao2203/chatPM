use http::{HeaderMap, HeaderValue};
use secrecy::SecretString;

use crate::ApiKey;

#[derive(Debug, Clone)]
pub struct Config {
    pub api_key: ApiKey,
}

impl async_openai::config::Config for Config {
    fn headers(&self) -> HeaderMap {
        let mut headers = HeaderMap::with_capacity(1);
        const PREFIX: &str = "Bearer ";
        let mut value = Vec::with_capacity(PREFIX.len() + self.api_key.expose_secret().len());
        value.extend_from_slice(PREFIX.as_bytes());
        value.extend_from_slice(self.api_key.expose_secret().as_bytes());
        headers.insert("Authorization", HeaderValue::from_bytes(&value).unwrap());
        headers
    }

    fn api_base(&self) -> &str {
        "https://api.deepseek.com"
    }

    fn api_key(&self) -> &SecretString {
        self.api_key.as_ref()
    }

    fn query(&self) -> Vec<(&str, &str)> {
        vec![]
    }

    fn url(&self, path: &str) -> String {
        format!("{}{}", self.api_base(), path)
    }
}
