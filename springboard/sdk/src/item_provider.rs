//! `NSItemProvider`: a value on its way through share, drag or copy, held as typed representations. Data can be
//! registered up front (`registerDataRepresentation`) or lazily through a [`DataProvider`]; a `Transferable` loads from
//! it (`loadTransferable`) through the best type it can read. Loading is synchronous here: Apple's completion handler
//! is the returned `Result`.

use crate::pasteboard_promise::{DataProvider, Promise};
use crate::transfer::{Transferable, Viewer};
use std::rc::Rc;

/// Why a load failed (`NSItemProvider.ErrorCode`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ItemProviderError {
    /// No registered type matches (`unavailableCoercion`).
    UnavailableCoercion,
    /// The provider had no data, or the data could not be read as the model (`unexpectedValueClass`).
    UnexpectedValue,
}

enum Source {
    Data(Vec<u8>),
    Lazy(Promise),
}

/// `NSItemProvider`.
#[derive(Default)]
pub struct NSItemProvider {
    reps: Vec<(String, Source)>,
}

impl NSItemProvider {
    pub fn new() -> Self {
        Self::default()
    }

    /// `init(object:)` for a `Transferable`: registers every representation another app may see.
    pub fn with_transferable<T: Transferable>(value: &T) -> Self {
        let reps = T::transfer_representations();
        let mut p = Self::new();
        for uti in reps.types_for(Viewer::Other) {
            if let Some(bytes) = reps.export(value, uti, Viewer::Other) {
                p.register_data_representation(uti, bytes);
            }
        }
        p
    }

    fn put(&mut self, uti: &str, src: Source) {
        self.reps.retain(|(t, _)| t != uti);
        self.reps.push((uti.to_string(), src));
    }

    /// `registerDataRepresentation(forTypeIdentifier:...)` with ready data.
    pub fn register_data_representation(&mut self, uti: &str, bytes: Vec<u8>) {
        self.put(uti, Source::Data(bytes));
    }

    /// `registerDataRepresentation` with a loader called the first time the type is loaded.
    pub fn register_lazy_representation(&mut self, uti: &str, provider: Rc<dyn DataProvider>) {
        self.put(uti, Source::Lazy(Promise::new(provider)));
    }

    /// `registeredTypeIdentifiers`, in registration order.
    pub fn registered_type_identifiers(&self) -> Vec<String> {
        self.reps.iter().map(|(t, _)| t.clone()).collect()
    }

    /// `hasItemConformingToTypeIdentifier`: a registered type is, or conforms to, `uti`.
    pub fn has_item_conforming_to(&self, uti: &str) -> bool {
        self.reps.iter().any(|(t, _)| t == uti || bifrost_ui::uttype::conforms(t, uti))
    }

    /// `loadDataRepresentation(forTypeIdentifier:)`.
    pub fn load_data_representation(&self, uti: &str) -> Result<Vec<u8>, ItemProviderError> {
        let (_, src) = self.reps.iter().find(|(t, _)| t == uti).ok_or(ItemProviderError::UnavailableCoercion)?;
        match src {
            Source::Data(d) => Ok(d.clone()),
            Source::Lazy(p) => p.resolve(uti).ok_or(ItemProviderError::UnexpectedValue),
        }
    }

    fn best_type<T: Transferable>(&self) -> Option<&'static str> {
        let offered = self.registered_type_identifiers();
        let offered: Vec<&str> = offered.iter().map(String::as_str).collect();
        T::transfer_representations().best_import(&offered)
    }

    /// `canLoadObject`: the model can read one of the registered types.
    pub fn can_load_transferable<T: Transferable>(&self) -> bool {
        self.best_type::<T>().is_some()
    }

    /// `loadTransferable(type:)`: reads the first type, in the model's preference order, that is registered.
    pub fn load_transferable<T: Transferable>(&self) -> Result<T, ItemProviderError> {
        let uti = self.best_type::<T>().ok_or(ItemProviderError::UnavailableCoercion)?;
        let bytes = self.load_data_representation(uti)?;
        T::transfer_representations().import(uti, &bytes).ok_or(ItemProviderError::UnexpectedValue)
    }
}
