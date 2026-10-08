//! Developing photos: the edits of a raw converter applied to a picture, shown live and exported.

pub mod geometry;
pub mod pipeline;

/// A picture as floating point RGB, row by row.
#[derive(Debug, Clone, PartialEq)]
pub struct Image {
    pub width: usize,
    pub height: usize,
    pub pixels: Vec<[f32; 3]>,
}

impl Image {
    pub fn new(width: usize, height: usize, pixels: Vec<[f32; 3]>) -> Image {
        assert_eq!(pixels.len(), width * height);
        Image {
            width,
            height,
            pixels,
        }
    }
}
