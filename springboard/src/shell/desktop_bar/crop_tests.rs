use super::crop_mask;
#[test]
fn a_clipped_card_keeps_its_mask_rows() {
    // a 4 x 3 mask, rows 0..3 hold 1, 2, 3
    let mask=[1,1,1,1, 2,2,2,2, 3,3,3,3];
    // clipped to 4 x 2 (the bottom row ran off the screen): rows 1 and 2 stay whole
    assert_eq!(crop_mask(&mask,4,4,2),vec![1,1,1,1,2,2,2,2]);
    // clipped to 3 x 3 (the right side ran off): the row stride is still 4
    assert_eq!(crop_mask(&mask,4,3,3),vec![1,1,1,2,2,2,3,3,3]);
}
