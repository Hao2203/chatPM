use anyhow::Result;
use futures_lite::{Stream, stream};

use crate::State;

pub async fn create_chat(state: &State, name: &str) -> Result<i64> {
    todo!()
}

pub async fn chat(
    state: &State,
    session_id: i64,
    message: &str,
) -> Result<impl Stream<Item = String>> {
    Ok(stream::empty())
}
