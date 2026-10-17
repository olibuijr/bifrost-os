//! App services for apps (the Services menu). An app declares each service as a `[[service]]` table in its
//! `manifest.toml` (`title`, `send_types`, `return_types`, `method`). The shell lists the services whose send types
//! match the selection and calls `service.<method>` on the provider's control socket with the selection:
//!
//! ```ignore
//! fn rpc_methods(&self) -> Vec<&'static str> { vec!["service.open_images"] }
//! fn rpc(&mut self, ctx: Option<&Ctx>, data_dir: &Path, method: &str, params: &Json) -> Result<Json, RpcError> {
//!     let call = ServiceCall::from_params(params).ok_or_else(|| RpcError::new(INVALID_PARAMS, "no selection"))?;
//!     // open call.paths ...; a service with return_types answers with `value_result(text)`
//!     Ok(Json::Obj(Vec::new()))
//! }
//! ```
//!
//! The types, the request and the codec live in `bifrost_ui::app_services`; this module is the app-facing surface.

pub use bifrost_ui::app_services::{is_image_path, is_viewable_path, value_result, Payload, ServiceCall, FILE_PATH_TEXT, FILE_URL, IMAGE, PDF, PLAIN_TEXT};
