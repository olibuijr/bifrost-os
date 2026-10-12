//! `bifrost_layer_v1` bindings generated from `protocol/bifrost-layer-v1.xml` with wayland-scanner: `client` for the
//! SDK, `server` for bifrost-wm, following the wayland-protocols crate layout (`__interfaces`, then
//! `generate_client_code!`/`generate_server_code!` in their own modules).
//!
//! The XML path is relative to the crate root (`CARGO_MANIFEST_DIR`), as wayland-scanner resolves it.
//! The transaction body of `commit` is the `codec` blob (`crate::codec::encode`/`decode`).
//!
//! Owner: wave G1 agent `layers-protocol`.

#[allow(
    dead_code,
    non_camel_case_types,
    unused_unsafe,
    unused_variables,
    non_upper_case_globals,
    non_snake_case,
    unused_imports,
    missing_docs,
    clippy::all
)]
mod generated {
    #[cfg(feature = "client")]
    pub mod client {
        //! Client-side API of `bifrost_layer_v1`.
        use wayland_client;
        use wayland_client::protocol::*;

        pub mod __interfaces {
            use wayland_client::protocol::__interfaces::*;
            wayland_scanner::generate_interfaces!("protocol/bifrost-layer-v1.xml");
        }
        use self::__interfaces::*;

        wayland_scanner::generate_client_code!("protocol/bifrost-layer-v1.xml");
    }

    #[cfg(feature = "server")]
    pub mod server {
        //! Server-side API of `bifrost_layer_v1`.
        use wayland_server;
        use wayland_server::protocol::*;

        pub mod __interfaces {
            use wayland_server::protocol::__interfaces::*;
            wayland_scanner::generate_interfaces!("protocol/bifrost-layer-v1.xml");
        }
        use self::__interfaces::*;

        wayland_scanner::generate_server_code!("protocol/bifrost-layer-v1.xml");
    }
}

#[cfg(feature = "client")]
pub use self::generated::client;

#[cfg(feature = "server")]
pub use self::generated::server;

#[cfg(all(test, feature = "client"))]
mod tests {
    use super::client::bifrost_layer_context_v1::{self, BifrostLayerContextV1};
    use super::client::bifrost_layer_manager_v1::{self, BifrostLayerManagerV1};

    #[test]
    fn client_bindings_name_the_requests_and_events() {
        // The generated request enums carry the XML's argument order: commit(transaction) and get_context(id, surface).
        let _ = std::mem::size_of::<bifrost_layer_context_v1::Event>();
        let _ = std::mem::size_of::<bifrost_layer_manager_v1::Request>();
        let _ = std::any::type_name::<BifrostLayerContextV1>();
        let _ = std::any::type_name::<BifrostLayerManagerV1>();
        assert_eq!(bifrost_layer_manager_v1::Error::AlreadyHasContext as u32, 0);
        assert_eq!(bifrost_layer_context_v1::Error::BadBuffer as u32, 1);
    }
}

#[cfg(all(test, feature = "server"))]
mod server_tests {
    use super::server::bifrost_layer_context_v1::{self, BifrostLayerContextV1};

    #[test]
    fn server_bindings_expose_the_context_interface() {
        let _ = std::any::type_name::<BifrostLayerContextV1>();
        assert_eq!(bifrost_layer_context_v1::Error::BadTransaction as u32, 0);
        assert_eq!(bifrost_layer_context_v1::Error::BadBuffer as u32, 1);
        assert_eq!(bifrost_layer_context_v1::EVT_PRESENTED_OPCODE, 0);
        assert_eq!(bifrost_layer_context_v1::EVT_ANIMATION_FINISHED_OPCODE, 1);
    }
}
