//! Pictures of a video track: H.264 decoded to RGB ([`video_decoder`]),
//! which frames the decoder can be given ([`order`]), made smaller
//! ([`fit`]), written as PNG ([`png_image`]), and which of them are kept for
//! a reader ([`frames`]). Nothing here reads a clock, a file or a socket.

pub mod fit;
pub mod frames;
pub mod order;
pub mod png_image;
pub mod video_decoder;

#[cfg(test)]
mod tests;
