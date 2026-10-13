//! PDF documents: file structure and the empty-password encryption handlers, the page tree, objects and content
//! streams, fonts (embedded programs and the standard fourteen), images, shadings and functions, colour spaces,
//! text extraction for reflowed reading, outlines and links, and page drawing through a drawing context.
//!
//! The file reader hands out [`doc::Doc`] and [`page::Page`]s; [`extract::Extractor`] turns a page into text lines
//! and images, and [`draw`] rasterises it through `cg::context`.

pub mod color;
pub mod crypt;
pub mod doc;
pub mod draw;
pub mod extract;
pub mod filter;
pub mod font;
pub mod function;
pub mod image;
pub mod inflate;
pub mod interpret;
pub mod obj;
pub mod page;
pub mod shading;
pub mod sink;
mod tables;
pub mod type1;
