use secrecy::{ExposeSecret, SecretString};

#[derive(Debug, Clone)]
pub struct ApiKey(SecretString);

impl ApiKey {
    pub fn new(api_key: impl AsRef<str>) -> Option<Self> {
        let api_key = api_key.as_ref().trim();

        for &b in api_key.as_bytes() {
            if !is_valid(b) {
                return None;
            }
        }

        Some(Self(SecretString::new(Box::from(api_key))))
    }

    pub fn expose_secret(&self) -> &str {
        self.0.expose_secret()
    }
}

impl AsRef<SecretString> for ApiKey {
    fn as_ref(&self) -> &SecretString {
        &self.0
    }
}

#[inline]
fn is_valid(b: u8) -> bool {
    b >= 32 && b != 127 || b == b'\t'
}
