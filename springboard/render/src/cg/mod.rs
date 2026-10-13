//! The one 2D drawing stack: paths and transforms, scan conversion and stroking, colours and colour spaces, paint
//! (gradients, shadings, functions, patterns), drawing contexts, bitmap images and image input/output, offscreen
//! layers and PDF documents.
pub mod color;
pub mod context;
pub mod image;
pub mod imageio;
pub mod paint;
pub mod path;
// pub mod pdf; (re-enabled by the PDF slice once cg/pdf/mod.rs exists)
pub mod raster;
pub mod reusable_layer;
pub mod surface;
