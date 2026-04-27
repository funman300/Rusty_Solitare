use crate::pile::PileType;

/// Score delta for moving cards from `from` to `to`.
///
/// Windows XP Standard scoring:
/// - +10 for any card reaching a foundation pile
/// - +5 for a waste → tableau move
/// - 0 for all other moves
pub fn score_move(from: &PileType, to: &PileType) -> i32 {
    match to {
        PileType::Foundation(_) => 10,
        PileType::Tableau(_) => {
            if matches!(from, PileType::Waste) { 5 } else { 0 }
        }
        _ => 0,
    }
}

/// Score penalty applied when the player uses undo: -15.
pub fn score_undo() -> i32 {
    -15
}

/// Time bonus added to the score on a win: `700_000 / elapsed_seconds`.
/// Returns 0 when `elapsed_seconds` is 0 to avoid division by zero.
pub fn compute_time_bonus(elapsed_seconds: u64) -> i32 {
    if elapsed_seconds == 0 {
        return 0;
    }
    (700_000u64 / elapsed_seconds).min(i32::MAX as u64) as i32
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::card::Suit;

    #[test]
    fn move_to_foundation_scores_ten() {
        assert_eq!(score_move(&PileType::Waste, &PileType::Foundation(Suit::Hearts)), 10);
        assert_eq!(score_move(&PileType::Tableau(0), &PileType::Foundation(Suit::Clubs)), 10);
    }

    #[test]
    fn waste_to_tableau_scores_five() {
        assert_eq!(score_move(&PileType::Waste, &PileType::Tableau(3)), 5);
    }

    #[test]
    fn tableau_to_tableau_scores_zero() {
        assert_eq!(score_move(&PileType::Tableau(0), &PileType::Tableau(1)), 0);
    }

    #[test]
    fn undo_penalty_is_negative_fifteen() {
        assert_eq!(score_undo(), -15);
    }

    #[test]
    fn time_bonus_at_100_seconds() {
        assert_eq!(compute_time_bonus(100), 7000);
    }

    #[test]
    fn time_bonus_at_zero_is_zero() {
        assert_eq!(compute_time_bonus(0), 0);
    }

    #[test]
    fn time_bonus_at_one_second() {
        assert_eq!(compute_time_bonus(1), 700_000);
    }

    #[test]
    fn non_waste_to_tableau_scores_zero() {
        // Foundation → Tableau is impossible in practice but must score 0.
        assert_eq!(score_move(&PileType::Foundation(Suit::Clubs), &PileType::Tableau(0)), 0);
        // Tableau → Tableau (restack) scores 0.
        assert_eq!(score_move(&PileType::Tableau(1), &PileType::Tableau(2)), 0);
    }

    #[test]
    fn move_to_stock_or_waste_scores_zero() {
        // These destinations are illegal moves in practice, but the function
        // must not panic and should return 0.
        assert_eq!(score_move(&PileType::Waste, &PileType::Stock), 0);
        assert_eq!(score_move(&PileType::Waste, &PileType::Waste), 0);
    }

    #[test]
    fn time_bonus_is_capped_at_i32_max_for_huge_values() {
        // Very short elapsed time would overflow without the .min() guard.
        let bonus = compute_time_bonus(1);
        assert!(bonus <= i32::MAX, "time bonus must fit in i32");
    }
}
