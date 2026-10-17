//! The 50-on palette: the hiragana chart in gojuon order, the grid a Japanese palette shows so a person can pick a kana
//! by its row (consonant) and column (vowel). Rows follow the chart: the vowel row, then k, s, t, n, h, m, y, r and w,
//! with ん on its own at the end. Cells the chart leaves empty are `None`. Pure model: it draws nothing and inserts
//! nothing; the palette view and the insertion go through the shell's text input.

/// The vowel of each column, in chart order.
pub const COLUMNS: [char; 5] = ['a', 'i', 'u', 'e', 'o'];

/// The consonant rows of the chart: the row label and its five cells (a, i, u, e, o).
pub const ROWS: [(&str, [Option<char>; 5]); 10] = [
    ("", [Some('あ'), Some('い'), Some('う'), Some('え'), Some('お')]),
    ("k", [Some('か'), Some('き'), Some('く'), Some('け'), Some('こ')]),
    ("s", [Some('さ'), Some('し'), Some('す'), Some('せ'), Some('そ')]),
    ("t", [Some('た'), Some('ち'), Some('つ'), Some('て'), Some('と')]),
    ("n", [Some('な'), Some('に'), Some('ぬ'), Some('ね'), Some('の')]),
    ("h", [Some('は'), Some('ひ'), Some('ふ'), Some('へ'), Some('ほ')]),
    ("m", [Some('ま'), Some('み'), Some('む'), Some('め'), Some('も')]),
    ("y", [Some('や'), None, Some('ゆ'), None, Some('よ')]),
    ("r", [Some('ら'), Some('り'), Some('る'), Some('れ'), Some('ろ')]),
    ("w", [Some('わ'), None, None, None, Some('を')]),
];

/// The syllabic nasal, shown on its own after the grid.
pub const NASAL: char = 'ん';

/// Every kana on the palette in chart order (the grid row by row, then ん).
pub fn all() -> Vec<char> {
    let mut out: Vec<char> = ROWS.iter().flat_map(|(_, cells)| cells.iter().flatten().copied()).collect();
    out.push(NASAL);
    out
}

/// The (row, column) of a kana in the grid, or None when it is not in the grid (ん is not).
pub fn position(kana: char) -> Option<(usize, usize)> {
    ROWS.iter().enumerate().find_map(|(r, (_, cells))| cells.iter().position(|cell| *cell == Some(kana)).map(|c| (r, c)))
}

/// The kana under a point in a palette `width` by `height` pixels, with its origin at the top left. The grid is five
/// columns by eleven rows: the ten chart rows, then ん alone on the last row under column 0. `None` outside the palette
/// or on a cell the chart leaves empty.
pub fn cell_at(x: f32, y: f32, width: f32, height: f32) -> Option<char> {
    if !(width > 0.0 && height > 0.0 && x >= 0.0 && y >= 0.0 && x < width && y < height) {
        return None;
    }
    let rows = ROWS.len() + 1;
    let col = ((x / width * COLUMNS.len() as f32) as usize).min(COLUMNS.len() - 1);
    let row = ((y / height * rows as f32) as usize).min(rows - 1);
    if row == ROWS.len() {
        return (col == 0).then_some(NASAL);
    }
    ROWS[row].1[col]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_chart_holds_forty_six_kana() {
        assert_eq!(all().len(), 46);
    }

    #[test]
    fn every_entry_is_a_hiragana_and_appears_once() {
        let kana = all();
        for c in &kana {
            assert!(('\u{3041}'..='\u{3093}').contains(c), "{c} is not hiragana");
        }
        let mut sorted = kana.clone();
        sorted.sort();
        sorted.dedup();
        assert_eq!(sorted.len(), kana.len());
    }

    #[test]
    fn a_kana_is_found_by_its_row_and_column() {
        assert_eq!(position('か'), Some((1, 0)));
        assert_eq!(position('を'), Some((9, 4)));
        assert_eq!(position('ゆ'), Some((7, 2)));
        assert_eq!(position(NASAL), None, "ん sits outside the grid");
        assert_eq!(position('x'), None);
    }

    #[test]
    fn a_point_hits_the_kana_under_it() {
        let (w, h) = (500.0, 1100.0);
        // each cell is 100 px wide and 100 px tall
        assert_eq!(cell_at(10.0, 10.0, w, h), Some('あ'));
        assert_eq!(cell_at(450.0, 950.0, w, h), Some('を'), "the bottom right cell is wo");
        assert_eq!(cell_at(150.0, 150.0, w, h), Some('き'), "row k, column i");
        assert_eq!(cell_at(10.0, 1050.0, w, h), Some(NASAL), "ん sits under column a on the last row");
        assert_eq!(cell_at(150.0, 1050.0, w, h), None, "nothing else sits on the last row");
    }

    #[test]
    fn a_point_on_an_empty_cell_or_outside_hits_nothing() {
        let (w, h) = (500.0, 1100.0);
        assert_eq!(cell_at(150.0, 750.0, w, h), None, "row y, column i is empty");
        assert_eq!(cell_at(-1.0, 10.0, w, h), None);
        assert_eq!(cell_at(10.0, 1100.0, w, h), None, "the far edge is outside");
        assert_eq!(cell_at(500.0, 10.0, w, h), None);
        assert_eq!(cell_at(10.0, 10.0, 0.0, h), None, "an empty palette has no cells");
    }

    #[test]
    fn empty_cells_are_where_the_chart_has_none() {
        assert_eq!(ROWS[7].1[1], None, "y row has no yi");
        assert_eq!(ROWS[9].1[1], None, "w row has no wi");
        assert_eq!(COLUMNS, ['a', 'i', 'u', 'e', 'o']);
    }
}
