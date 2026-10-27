use super::step_row;

#[test]
fn arrows_walk_the_rows_and_stop_at_the_ends() {
    let rows = [(0, 0), (0, 1), (1, 0)];
    assert_eq!(step_row(&rows, None, 1), Some((0, 0)));
    assert_eq!(step_row(&rows, None, -1), Some((1, 0)));
    assert_eq!(step_row(&rows, Some((0, 0)), 1), Some((0, 1)));
    assert_eq!(step_row(&rows, Some((1, 0)), 1), Some((1, 0)));
    assert_eq!(step_row(&rows, Some((0, 0)), -1), Some((0, 0)));
}

#[test]
fn no_rows_means_no_selection() {
    assert_eq!(step_row(&[], None, 1), None);
    assert_eq!(step_row(&[], Some((0, 0)), -1), None);
}
