use thiserror::Error;

/// All reasons a game move can be rejected.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum MoveError {
    #[error("invalid source pile")]
    InvalidSource,
    #[error("invalid destination pile")]
    InvalidDestination,
    #[error("source pile is empty")]
    EmptySource,
    #[error("move violates rules: {0}")]
    RuleViolation(String),
    #[error("undo stack is empty")]
    UndoStackEmpty,
    #[error("game is already won")]
    GameAlreadyWon,
    #[error("stock and waste are both empty")]
    StockEmpty,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rule_violation_includes_message() {
        let e = MoveError::RuleViolation("king only on empty".into());
        assert!(e.to_string().contains("king only on empty"));
    }

    #[test]
    fn undo_stack_empty_has_non_empty_message() {
        assert!(!MoveError::UndoStackEmpty.to_string().is_empty());
    }
}
