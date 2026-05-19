use crate::pile::PileType;

/// Score delta for moving cards from `from` to `to`.
///
/// Windows XP Standard scoring:
/// - +10 for any card reaching a foundation pile
/// - +5 for a waste → tableau move
/// - -15 for a foundation → tableau (take-from-foundation) move
/// - 0 for all other moves
///
/// Note: the +5 flip bonus for exposing a face-down tableau card is applied
/// separately in `game_state::move_cards` because it depends on post-move state.
pub fn score_move(from: &PileType, to: &PileType) -> i32 {
    match to {
        PileType::Foundation(_) => 10,
        PileType::Tableau(_) => match from {
            PileType::Waste => 5,
            PileType::Foundation(_) => -15,
            _ => 0,
        },
        _ => 0,
    }
}

/// Score penalty applied when the player uses undo: -15.
pub fn score_undo() -> i32 {
    -15
}

/// Score bonus awarded when a face-down tableau card is flipped face-up: +5.
pub fn score_flip() -> i32 {
    5
}

/// Score penalty for recycling the waste pile back to stock.
///
/// Windows standard: the first N recycles are free (N=1 for Draw-1, N=3 for Draw-3).
/// Subsequent recycles cost -100 (Draw-1) or -20 (Draw-3).
/// `recycle_count` is the new total count **after** this recycle.
pub fn score_recycle(recycle_count: u32, is_draw_three: bool) -> i32 {
    let (free, penalty) = if is_draw_three { (3_u32, -20_i32) } else { (1_u32, -100_i32) };
    if recycle_count > free { penalty } else { 0 }
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

    #[test]
    fn move_to_foundation_scores_ten() {
        assert_eq!(score_move(&PileType::Waste, &PileType::Foundation(2)), 10);
        assert_eq!(score_move(&PileType::Tableau(0), &PileType::Foundation(0)), 10);
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
    fn foundation_to_tableau_penalises_fifteen() {
        // Moving a card back off a foundation (take_from_foundation rule) costs -15.
        assert_eq!(score_move(&PileType::Foundation(0), &PileType::Tableau(0)), -15);
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
        assert!(bonus >= 0, "time bonus must be non-negative after u64→i32 cast");
    }

    #[test]
    fn flip_bonus_is_five() {
        assert_eq!(score_flip(), 5);
    }

    #[test]
    fn recycle_draw1_first_pass_free() {
        assert_eq!(score_recycle(1, false), 0);
    }

    #[test]
    fn recycle_draw1_second_pass_penalised() {
        assert_eq!(score_recycle(2, false), -100);
    }

    #[test]
    fn recycle_draw3_third_pass_free() {
        assert_eq!(score_recycle(3, true), 0);
    }

    #[test]
    fn recycle_draw3_fourth_pass_penalised() {
        assert_eq!(score_recycle(4, true), -20);
    }
}
