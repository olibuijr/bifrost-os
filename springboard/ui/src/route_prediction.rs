//! Route suggestions for the sound output picker. Suggestions are the destinations an account has used before, most
//! used first and, among equally used ones, the most recently used first. The history is the list of destination
//! identifiers the picker chose, oldest first, so the model needs no clock.

use crate::route_picker::Destination;

/// Returns up to `limit` destinations from `destinations` that appear in `history`, ranked as described above.
/// A destination never chosen is not suggested. Identifiers in `history` that match no destination are ignored.
pub fn suggest<'a>(destinations: &'a [Destination], history: &[&str], limit: usize) -> Vec<&'a Destination> {
    let mut ranked: Vec<(usize, usize, &Destination)> = destinations
        .iter()
        .filter_map(|destination| {
            let count = history.iter().filter(|id| **id == destination.id).count();
            let last = history.iter().rposition(|id| *id == destination.id)?;
            Some((count, last, destination))
        })
        .collect();
    ranked.sort_by(|a, b| b.0.cmp(&a.0).then(b.1.cmp(&a.1)));
    ranked.into_iter().take(limit).map(|(_, _, destination)| destination).collect()
}
