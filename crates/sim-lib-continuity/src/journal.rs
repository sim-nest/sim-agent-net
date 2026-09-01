use crate::{
    ContinuityEvent, ContinuityPlan, ContinuityRefusal, ContinuityState, ContinuityTurn, apply,
};

/// One fenced append row.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct JournalRow {
    /// Expected prior sequence count.
    pub expected_len: u64,
    /// Accepted canonical turn.
    pub turn: ContinuityTurn,
}
/// Journal adapter supplied by a composition owner.
pub trait ContinuityJournal {
    /// Reads canonical accepted turns.
    fn turns(&self) -> &[ContinuityTurn];
    /// Appends only if the caller's fence still matches.
    fn append_fenced(&mut self, row: JournalRow) -> Result<(), JournalError>;
}
/// Fenced journal failure.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum JournalError {
    /// Another writer advanced the journal.
    FenceConflict,
    /// Reducer refusal.
    Refused(ContinuityRefusal),
    /// Supplied cache was not derived from this journal.
    StateMismatch,
}
/// Test and embedded adapter; production compositions may adapt the delivered journal.
#[derive(Clone, Debug, Default)]
pub struct MemoryJournal {
    rows: Vec<ContinuityTurn>,
}
impl ContinuityJournal for MemoryJournal {
    fn turns(&self) -> &[ContinuityTurn] {
        &self.rows
    }
    fn append_fenced(&mut self, row: JournalRow) -> Result<(), JournalError> {
        if self.rows.len() as u64 != row.expected_len {
            return Err(JournalError::FenceConflict);
        }
        self.rows.push(row.turn);
        Ok(())
    }
}
impl MemoryJournal {
    /// Reduces and atomically appends one accepted transition.
    pub fn accept(
        &mut self,
        plan: &ContinuityPlan,
        state: &ContinuityState,
        event: ContinuityEvent,
    ) -> Result<ContinuityState, JournalError> {
        let rebuilt = crate::rebuild(plan, self.turns()).map_err(JournalError::Refused)?;
        if &rebuilt != state {
            return Err(JournalError::StateMismatch);
        }
        let next = apply(plan, state, event).map_err(JournalError::Refused)?;
        if next == *state {
            return Ok(next);
        }
        let turn = next
            .turns
            .last()
            .expect("accepted transition has turn")
            .clone();
        self.append_fenced(JournalRow {
            expected_len: self.rows.len() as u64,
            turn,
        })?;
        Ok(next)
    }
}
