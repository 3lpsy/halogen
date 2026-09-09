use super::reordered;

#[test]
fn reordered_moves_within_range_and_renumbers() {
    // Move index 0 to index 2 → [b, c, a, d].
    assert_eq!(
        reordered(&[10, 20, 30, 40], 0, 2),
        Some(vec![20, 30, 10, 40])
    );
    // Move last to front.
    assert_eq!(reordered(&[10, 20, 30], 2, 0), Some(vec![30, 10, 20]));
}

#[test]
fn reordered_clamps_out_of_range_target() {
    // `to` past the end clamps to the last slot.
    assert_eq!(reordered(&[10, 20, 30], 0, 99), Some(vec![20, 30, 10]));
}

#[test]
fn reordered_no_op_cases_return_none() {
    // Same index (after clamp) is a no-op.
    assert_eq!(reordered(&[10, 20, 30], 1, 1), None);
    // Clamped target equals from.
    assert_eq!(reordered(&[10, 20, 30], 2, 99), None);
    // from out of range.
    assert_eq!(reordered(&[10, 20], 5, 0), None);
    // Empty list.
    assert_eq!(reordered(&[], 0, 0), None);
}
