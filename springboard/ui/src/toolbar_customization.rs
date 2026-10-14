//! Toolbar customization: which toolbar items the person shows, and the order they appear in.
//!
//! The app offers a set of item ids in a default order and names the items that cannot be hidden, such as the button
//! that closes a sheet. The person may hide any other item, show it again, move it to another position, or reset the
//! toolbar to the app's default. The model stores only ids; the app builds the toolbar from [`ToolbarCustomization::visible`]
//! and keeps the items' labels and actions. The rules here are this module's own.

/// Why a customization change was refused.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    /// The id is not one the app offers.
    UnknownItem,
    /// The item is required and cannot be hidden.
    Required,
}

/// The shown items and their order for one toolbar.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ToolbarCustomization {
    default_order: Vec<u32>,
    required: Vec<u32>,
    order: Vec<u32>,
    hidden: Vec<u32>,
}

impl ToolbarCustomization {
    /// A toolbar offering `default_order`, with the `required` items always shown. Ids in `required` that the app does
    /// not offer are ignored.
    pub fn new(default_order: Vec<u32>, required: Vec<u32>) -> Self {
        let required = required.into_iter().filter(|id| default_order.contains(id)).collect();
        let order = default_order.clone();
        ToolbarCustomization { default_order, required, order, hidden: Vec::new() }
    }

    /// Every offered item in the current order, shown or hidden.
    pub fn order(&self) -> &[u32] {
        &self.order
    }

    /// The shown items in the current order: the order without the hidden items.
    pub fn visible(&self) -> Vec<u32> {
        self.order.iter().copied().filter(|id| !self.hidden.contains(id)).collect()
    }

    /// True when the item is offered and shown.
    pub fn is_shown(&self, id: u32) -> bool {
        self.order.contains(&id) && !self.hidden.contains(&id)
    }

    /// Hides the item `id`. Hiding an item that is already hidden changes nothing.
    pub fn hide(&mut self, id: u32) -> Result<(), Error> {
        self.check_offered(id)?;
        if self.required.contains(&id) {
            return Err(Error::Required);
        }
        if !self.hidden.contains(&id) {
            self.hidden.push(id);
        }
        Ok(())
    }

    /// Shows the item `id` again, in the place it keeps in the order.
    pub fn show(&mut self, id: u32) -> Result<(), Error> {
        self.check_offered(id)?;
        self.hidden.retain(|&h| h != id);
        Ok(())
    }

    /// Moves the item `id` to position `to` in the order, counting hidden items too. A position past the end moves
    /// the item to the end.
    pub fn move_item(&mut self, id: u32, to: usize) -> Result<(), Error> {
        self.check_offered(id)?;
        self.order.retain(|&o| o != id);
        let at = to.min(self.order.len());
        self.order.insert(at, id);
        Ok(())
    }

    /// Shows every item and restores the default order.
    pub fn reset(&mut self) {
        self.order = self.default_order.clone();
        self.hidden.clear();
    }

    fn check_offered(&self, id: u32) -> Result<(), Error> {
        if self.default_order.contains(&id) {
            Ok(())
        } else {
            Err(Error::UnknownItem)
        }
    }
}
