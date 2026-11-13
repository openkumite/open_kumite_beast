//! The address map of the imported game's data: CMN.ACT's clip table and the scripts' clip pointers hold addresses in the
//! game's memory; a pointer into the program image maps back to a file offset of the image the converter keeps
//! (`Br1Res::image_clips`). The only place the engine names those addresses.
//!
//! State: none

/// The address of the image's file offset 0 in the game's memory (the text follows a 0x800-byte header).
pub const IMAGE_BASE: u32 = 0x8011_F800;
/// The address the image's text starts at: a pointer below it is not into the image.
const TEXT_BASE: u32 = 0x8012_0000;

/// The image file offset a data pointer `w` names, by the console's RAM mirrors (kuseg / kseg0 / kseg1: the 2 MB of
/// RAM mirrored over the first 8 MB), when it points into the image's text and below `image_len`.
pub fn image_offset(w: u32, image_len: usize) -> Option<usize> {
    let phys = w & 0x1FFF_FFFF;
    let ram = 0x8000_0000 | (phys & 0x1F_FFFF);
    (phys < 0x80_0000 && ram >= TEXT_BASE).then(|| (ram - IMAGE_BASE) as usize).filter(|&o| o < image_len)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mirrors_and_bounds() {
        assert_eq!(image_offset(IMAGE_BASE + 0x800, 0x1000), Some(0x800));
        assert_eq!(image_offset((IMAGE_BASE + 0x800) & 0x1FFF_FFFF, 0x1000), Some(0x800));
        assert_eq!(image_offset(IMAGE_BASE + 0x7FF, 0x1000), None);
        assert_eq!(image_offset(IMAGE_BASE + 0x1000, 0x1000), None);
        assert_eq!(image_offset(0x0072_0DE6, 0x60_0000), Some(0x15E6));
    }
}
