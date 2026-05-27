use anyhow::{Result, anyhow};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReasoningEffort {
    High,
    Max,
}

impl ReasoningEffort {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::High => "high",
            Self::Max => "max",
        }
    }

    pub fn parse(value: &str) -> Result<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "high" => Ok(Self::High),
            "max" => Ok(Self::Max),
            other => Err(anyhow!(
                "invalid reasoning_effort '{}', expected one of: high, max",
                other
            )),
        }
    }
}
