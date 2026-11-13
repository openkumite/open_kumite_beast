//! CD-DA audio as the converter exports it (`.pcm`, engine addition): the imported game's CD audio track
//! (`br1/audio/cdda/track02.pcm`), the disc's 16-bit samples unchanged behind a small header.
//!
//! Version 1: `b"BR2PCM\0"`, u8 version 1, u32 LE sample rate, u16 LE channels (1 or 2), u16 0, u32 LE frames N,
//! u32 0 (24 bytes), then N × channels 16-bit little-endian samples, interleaved. Written by ok_br1_converter's
//! `runtime::Converter` (44100 Hz stereo, the track from its INDEX 01).

use super::music::Music;

/// The file's first bytes (a historical format id, kept so that files of earlier builds stay readable: the converted data writes it).
pub const MAGIC: &[u8; 7] = b"BR2PCM\0";
pub const HEADER: usize = 24;

/// A `.pcm` file → interleaved stereo (a mono file's samples doubled); Err on a damaged header or a short body.
pub fn decode(bytes: &[u8]) -> Result<Music, String> {
    if bytes.len() < HEADER || !bytes.starts_with(MAGIC) {
        return Err("not a .pcm file".into());
    }
    let u16_at = |o: usize| u16::from_le_bytes([bytes[o], bytes[o + 1]]);
    let u32_at = |o: usize| u32::from_le_bytes([bytes[o], bytes[o + 1], bytes[o + 2], bytes[o + 3]]);
    let (version, rate, channels, frames) = (bytes[7], u32_at(8), u16_at(12) as usize, u32_at(16) as usize);
    if version != 1 {
        return Err(format!(".pcm version {version} (this build reads 1)"));
    }
    if !(1..=2).contains(&channels) || !(8000..=96_000).contains(&rate) {
        return Err(format!(".pcm: {channels} channels at {rate} Hz"));
    }
    let body = &bytes[HEADER..];
    let need = frames.checked_mul(channels * 2).ok_or(".pcm: too many frames")?;
    if body.len() < need {
        return Err(format!(".pcm: {} bytes of samples, the header says {need}", body.len()));
    }
    let samples = body[..need].as_chunks::<2>().0.iter().map(|b| i16::from_le_bytes(*b));
    let pcm = if channels == 2 { samples.collect() } else { samples.flat_map(|s| [s, s]).collect() };
    Ok(Music { pcm, rate })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn file(rate: u32, channels: u16, samples: &[i16]) -> Vec<u8> {
        let mut b = MAGIC.to_vec();
        b.push(1);
        b.extend(rate.to_le_bytes());
        b.extend(channels.to_le_bytes());
        b.extend([0, 0]);
        b.extend((samples.len() as u32 / channels as u32).to_le_bytes());
        b.extend([0; 4]);
        b.extend(samples.iter().flat_map(|s| s.to_le_bytes()));
        b
    }

    #[test]
    fn stereo_mono_and_damaged_files() {
        let m = decode(&file(44100, 2, &[1, -2, 300, -32768])).unwrap();
        assert_eq!((m.rate, m.pcm), (44100, vec![1, -2, 300, -32768]));
        let m = decode(&file(22050, 1, &[5, -6])).unwrap();
        assert_eq!((m.rate, m.pcm), (22050, vec![5, 5, -6, -6]));
        // a body shorter than the header says, another version, a bad channel count, not a .pcm
        let mut short = file(44100, 2, &[1, 2]);
        short.pop();
        assert!(decode(&short).is_err());
        let mut v2 = file(44100, 2, &[1, 2]);
        v2[7] = 2;
        assert!(decode(&v2).err().unwrap().contains("version 2"));
        assert!(decode(&file(44100, 3, &[1, 2, 3])).is_err());
        assert!(decode(b"BR2XA\0").is_err());
        // the converter's header (ok_br1_converter runtime::pcm_header) is this one
        assert_eq!(file(44100, 2, &[]).len(), HEADER);
    }
}
