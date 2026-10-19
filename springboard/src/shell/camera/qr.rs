//! The QR decoder for the Camera's scan overlay. The code moved to `bifrost_render::qr_decode` so apps can
//! scan images too (`bifrost_app::image_detect`); this module keeps the old path for the viewfinder.
pub use bifrost_render::qr_decode::{decode_luma, luma_of};
