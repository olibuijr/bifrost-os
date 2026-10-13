//! The 2D drawing stack. Each submodule is owned by one part of the stack: `color` (colours and colour spaces) and
//! `paint` (gradients, shadings, functions, patterns) live here; the path, raster, context, image and document
//! modules are added alongside by their owners.
pub mod color;
pub mod paint;
