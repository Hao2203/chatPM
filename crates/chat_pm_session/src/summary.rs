use crate::TurnId;

#[derive(Debug, Clone)]
pub struct Summary {
    pub content: String,
    pub last_turn_id: TurnId,
}
