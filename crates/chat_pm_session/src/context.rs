use crate::{memory::Memory, summary::Summary};

#[derive(Debug, Clone)]
pub struct Context {
    pub summary: Option<Summary>,
    pub recent_memory: Vec<Memory>,
}
