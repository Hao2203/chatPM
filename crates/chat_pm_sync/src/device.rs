use serde::{Deserialize, Deserializer, Serialize, Serializer};

/// 256-bit 设备唯一标识符。
///
/// 内部以 `[u8; 32]` 存储，序列化为 64 位 hex 字符串。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct DeviceId([u8; 32]);

impl DeviceId {
    /// 生成一个新的随机设备 ID（使用两个 UUID v4 提供约 244 位随机性）。
    pub fn generate() -> Self {
        let high = uuid::Uuid::new_v4().as_u128();
        let low = uuid::Uuid::new_v4().as_u128();
        let mut bytes = [0u8; 32];
        bytes[..16].copy_from_slice(&high.to_be_bytes());
        bytes[16..].copy_from_slice(&low.to_be_bytes());
        Self(bytes)
    }

    /// 从 64 hex 字符解析。
    pub fn from_hex(hex: &str) -> Result<Self, DeviceIdError> {
        let trimmed = hex.trim();
        if trimmed.len() != 64 || !trimmed.chars().all(|c| c.is_ascii_hexdigit()) {
            return Err(DeviceIdError::InvalidFormat);
        }
        let mut bytes = [0u8; 32];
        for i in 0..32 {
            bytes[i] = u8::from_str_radix(&trimmed[2 * i..2 * i + 2], 16)
                .map_err(|_| DeviceIdError::InvalidFormat)?;
        }
        Ok(Self(bytes))
    }

    /// 从 32 字节数组构造。
    pub fn from_bytes(bytes: [u8; 32]) -> Self {
        Self(bytes)
    }

    /// 从 ed25519 私钥派生设备标识（= 公钥的 32 字节）。
    pub fn from_secret_key(secret_key_bytes: &[u8; 32]) -> Self {
        let signing = ed25519_dalek::SigningKey::from_bytes(secret_key_bytes);
        let verifying = signing.verifying_key();
        Self(verifying.to_bytes())
    }

    /// 生成新的随机设备身份。
    ///
    /// 返回 `(DeviceId, 私钥_bytes)`。私钥应持久化存储以供后续恢复。
    pub fn generate_identity() -> (Self, [u8; 32]) {
        let mut bytes = [0u8; 32];
        bytes[..16].copy_from_slice(&uuid::Uuid::new_v4().into_bytes());
        bytes[16..].copy_from_slice(&uuid::Uuid::new_v4().into_bytes());
        let signing = ed25519_dalek::SigningKey::from_bytes(&bytes);
        let key_bytes = signing.to_bytes();
        (Self::from_secret_key(&key_bytes), key_bytes)
    }

    /// 返回 hex 编码的 64 字符字符串。
    pub fn to_hex(&self) -> String {
        self.0.iter().map(|b| format!("{b:02x}")).collect()
    }

    pub fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }

    pub fn into_inner(self) -> [u8; 32] {
        self.0
    }
}

impl std::fmt::Display for DeviceId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        for byte in &self.0 {
            write!(f, "{byte:02x}")?;
        }
        Ok(())
    }
}

// 序列化为 hex 字符串
impl Serialize for DeviceId {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.to_hex())
    }
}

impl<'de> Deserialize<'de> for DeviceId {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let s = <&str>::deserialize(deserializer)?;
        Self::from_hex(s).map_err(serde::de::Error::custom)
    }
}

// ── Error ──────────────────────────────────────────────────────────────

/// DeviceId 解析错误。
#[derive(Debug, Clone, thiserror::Error)]
pub enum DeviceIdError {
    #[error("Device ID 必须是 64 hex 字符（256 位）")]
    InvalidFormat,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_generate_and_hex_roundtrip() {
        let id = DeviceId::generate();
        let hex = id.to_hex();
        assert_eq!(hex.len(), 64);
        assert!(hex.chars().all(|c| c.is_ascii_hexdigit()));

        let parsed = DeviceId::from_hex(&hex).unwrap();
        assert_eq!(id, parsed);
    }

    #[test]
    fn test_serde_roundtrip() {
        let id = DeviceId::generate();
        let json = serde_json::to_string(&id).unwrap();
        let parsed: DeviceId = serde_json::from_str(&json).unwrap();
        assert_eq!(id, parsed);
        // JSON should be quoted hex string
        assert_eq!(json, format!("\"{}\"", id.to_hex()));
    }

    #[test]
    fn test_invalid_hex() {
        assert!(DeviceId::from_hex("short").is_err());
        assert!(
            DeviceId::from_hex("gggggggggggggggggggggggggggggggggggggggggggggggggggggggggggggggg")
                .is_err()
        );
    }

    #[test]
    fn test_display() {
        let bytes = [0u8; 32];
        let id = DeviceId(bytes);
        assert_eq!(
            id.to_string(),
            "0000000000000000000000000000000000000000000000000000000000000000"
        );
    }
}
