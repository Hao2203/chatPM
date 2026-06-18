use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// 知识库的唯一标识符。
///
/// 使用 UUID v7（时间有序），可以在客户端生成，无需数据库往返。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct KnowledgeBaseId(Uuid);

impl KnowledgeBaseId {
    /// 生成一个新的知识库 ID。
    pub fn new() -> Self {
        Self(Uuid::now_v7())
    }

    /// 从已有的 Uuid 构造。
    pub fn from_uuid(uuid: Uuid) -> Self {
        Self(uuid)
    }

    /// 获取内部 Uuid。
    pub fn as_uuid(&self) -> Uuid {
        self.0
    }
}

impl std::fmt::Display for KnowledgeBaseId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl std::str::FromStr for KnowledgeBaseId {
    type Err = uuid::Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Uuid::parse_str(s).map(Self)
    }
}

impl Default for KnowledgeBaseId {
    fn default() -> Self {
        Self::new()
    }
}

/// 知识库的人类可读名称。
///
/// 约束：非空、长度不超过 128 字符。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct KnowledgeBaseName(String);

impl KnowledgeBaseName {
    /// 从字符串创建名称，自动去除首尾空白。
    pub fn new(name: impl AsRef<str>) -> Self {
        Self(name.as_ref().trim().to_string())
    }

    /// 检查名称是否有效。
    pub fn is_valid(&self) -> bool {
        !self.0.is_empty() && self.0.len() <= 128
    }

    /// 获取名称的字符串引用。
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// 消耗并返回内部字符串。
    pub fn into_inner(self) -> String {
        self.0
    }
}

impl std::fmt::Display for KnowledgeBaseName {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// 知识库的元数据（不包含向量数据，向量数据在 EdgeShard 中）。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KnowledgeBase {
    /// 唯一标识符。
    pub id: KnowledgeBaseId,
    /// 人类可读名称。
    pub name: KnowledgeBaseName,
    /// 文档总数。
    pub document_count: usize,
    /// 文本块总数。
    pub total_chunks: usize,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn knowledge_base_name_empty_is_invalid() {
        let name = KnowledgeBaseName::new("");
        assert!(!name.is_valid());
    }

    #[test]
    fn knowledge_base_name_whitespace_only_is_invalid() {
        let name = KnowledgeBaseName::new("   ");
        assert!(!name.is_valid());
    }

    #[test]
    fn knowledge_base_name_normal_is_valid() {
        let name = KnowledgeBaseName::new("我的资料库");
        assert!(name.is_valid());
    }

    #[test]
    fn knowledge_base_name_trims_whitespace() {
        let name = KnowledgeBaseName::new("  技术文档  ");
        assert_eq!(name.as_str(), "技术文档");
    }

    #[test]
    fn knowledge_base_name_too_long() {
        let long_name = "a".repeat(129);
        let name = KnowledgeBaseName::new(&long_name);
        assert!(!name.is_valid());
    }

    #[test]
    fn knowledge_base_id_new_generates_unique() {
        let id1 = KnowledgeBaseId::new();
        let id2 = KnowledgeBaseId::new();
        assert_ne!(id1, id2);
    }

    #[test]
    fn knowledge_base_id_roundtrip_display_parse() {
        let id = KnowledgeBaseId::new();
        let s = id.to_string();
        let parsed: KnowledgeBaseId = s.parse().unwrap();
        assert_eq!(id, parsed);
    }
}
