//! Settings > General > Login Items: the START ORDER group, which sets the order the login items start in.
//! `pages::spec` adds the group under OPEN AT LOGIN; the shell moves the apps (`Shell::login_items_move`) and the
//! order is saved with the list in the `login.items` pref.
use super::list::{Group, Item};
use super::pages::Action;

/// The START ORDER group: one row per login item in start order, each showing its place. Choosing a row moves that
/// app one place earlier. None when there is nothing to order (fewer than two items).
pub fn order_group(items: &[(String, String)]) -> Option<Group> {
    if items.len() < 2 {
        return None;
    }
    let rows = items
        .iter()
        .enumerate()
        .map(|(i, (id, name))| Item::new(name.as_str()).value(format!("{} of {}", i + 1, items.len())).action(Action::LoginItemMove(id.clone())))
        .collect();
    Some(Group::new(rows).header("START ORDER").footer("Apps start in this order when you log in. Choose an app to move it one place earlier."))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn item(id: &str, name: &str) -> (String, String) {
        (id.to_string(), name.to_string())
    }

    #[test]
    fn one_or_no_login_item_has_no_order_to_set() {
        assert!(order_group(&[]).is_none());
        assert!(order_group(&[item("a", "Alpha")]).is_none());
    }

    #[test]
    fn every_login_item_gets_its_place_in_the_order() {
        let g = order_group(&[item("a", "Alpha"), item("b", "Beta"), item("c", "Gamma")]).expect("three items");
        assert_eq!(g.header.as_deref(), Some("START ORDER"));
        assert_eq!(g.items.len(), 3);
        assert_eq!(g.items[0].label, "Alpha");
        assert_eq!(g.items[0].value, "1 of 3");
        assert_eq!(g.items[2].value, "3 of 3");
    }
}
