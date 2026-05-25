use serde::{Deserialize, Serialize};

/// 同步链的准入凭证。
///
/// 拥有相同 ticket 的设备同步相同的记录。
/// 第一台设备创建文档获得 ticket，后续设备凭 ticket 加入同一同步链。
///
/// 不是裸 `String`，防止混入其他字符串参数；
/// 调用方无法凭空构造——只能通过 `create_doc()` 获取。
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct DocTicket(String);

impl DocTicket {
    pub fn from_string(s: String) -> Self {
        Self(s)
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    pub fn into_string(self) -> String {
        self.0
    }
}

impl std::fmt::Display for DocTicket {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl Serialize for DocTicket {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.0)
    }
}

impl<'de> Deserialize<'de> for DocTicket {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let s = String::deserialize(deserializer)?;
        Ok(Self(s))
    }
}
