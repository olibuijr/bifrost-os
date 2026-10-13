//! The one 2D drawing stack: paths and transforms, scan conversion and stroking, colours and colour spaces, and
//! paint (gradients, shadings, functions, patterns). Context, image and document modules join here.
pub mod color;
pub mod paint;
pub mod path;
pub mod raster;
pub mod surface;
