//! Music: the XA tracks and the movies' audio as the converter exports them, decoded to interleaved stereo: the
//! disc's CD-XA ADPCM (`.xa`, `formats::xa`), the imported game's CD-DA track (`.pcm`, `formats::pcm`) or, from older
//! exports, MP3 (with symphonia).

/// Decoded music (interleaved stereo, 16-bit as the CD-XA streams it replaces: half the memory of f32).
pub struct Music {
    pub pcm: Vec<i16>,
    pub rate: u32,
}

/// XA track `track`'s file in the export (the disc's `BR2_MCnn.XA` channel `track % 8`), by its MP3 name:
/// `Assets::load_music` reads the `.xa` beside it first.
pub fn track_file(track: u8) -> String {
    format!("audio/music/BR2_MC{:02}_ch{}.mp3", track / 8 + 1, track % 8)
}

/// A music file (`Assets::load_music`'s bytes) decoded: a `.xa` file or (the imported game's CD audio) a `.pcm` file by
/// its magic, else MP3.
pub fn decode(bytes: impl AsRef<[u8]> + Send + Sync + 'static) -> Option<Music> {
    if bytes.as_ref().starts_with(super::xa::MAGIC) {
        return super::xa::decode(bytes.as_ref()).map_err(|e| log::warn!("{e}")).ok();
    }
    if bytes.as_ref().starts_with(super::pcm::MAGIC) {
        return super::pcm::decode(bytes.as_ref()).map_err(|e| log::warn!("{e}")).ok();
    }
    decode_mp3(bytes)
}

/// MP3 → interleaved stereo (the XA music of older exports).
pub fn decode_mp3(bytes: impl AsRef<[u8]> + Send + Sync + 'static) -> Option<Music> {
    use symphonia::core::{audio::SampleBuffer, codecs::DecoderOptions, formats::FormatOptions, io::MediaSourceStream, meta::MetadataOptions, probe::Hint};
    let mss = MediaSourceStream::new(Box::new(std::io::Cursor::new(bytes)), Default::default());
    let mut hint = Hint::new();
    hint.with_extension("mp3");
    let probed = symphonia::default::get_probe().format(&hint, mss, &FormatOptions::default(), &MetadataOptions::default()).ok()?;
    let mut format = probed.format;
    let track = format.default_track()?;
    let id = track.id;
    let mut dec = symphonia::default::get_codecs().make(&track.codec_params, &DecoderOptions::default()).ok()?;
    let mut rate = track.codec_params.sample_rate.unwrap_or(44100);
    // the whole track at once when the stream says how long it is (no doubling growth: a 3-minute track would sit in
    // a block twice its size); capped so a bad header can't ask for more than 20 minutes
    let frames = track.codec_params.n_frames.unwrap_or(0).min(20 * 60 * 48_000) as usize;
    let mut pcm = Vec::with_capacity(frames * 2);
    while let Ok(packet) = format.next_packet() {
        if packet.track_id() != id {
            continue;
        }
        let Ok(buf) = dec.decode(&packet) else { continue };
        let spec = *buf.spec();
        rate = spec.rate;
        let ch = spec.channels.count();
        let mut sb = SampleBuffer::<f32>::new(buf.capacity() as u64, spec);
        sb.copy_interleaved_ref(buf);
        for frame in sb.samples().chunks(ch) {
            let l = frame[0];
            let r = if ch > 1 { frame[1] } else { l };
            pcm.push(to_i16(l));
            pcm.push(to_i16(r));
        }
    }
    pcm.shrink_to_fit();
    Some(Music { pcm, rate })
}

/// A decoded sample (nominally -1..1) as 16-bit.
fn to_i16(v: f32) -> i16 {
    (v * 32767.0).round().clamp(-32768.0, 32767.0) as i16
}

#[cfg(test)]
mod tests {
    use super::{decode, to_i16};

    #[test]
    fn samples_to_16_bit() {
        assert_eq!([0.0, 1.0, -1.0, 0.5, 2.0, -2.0, f32::NAN].map(to_i16), [0, 32767, -32767, 16384, 32767, -32768, 0]);
    }

    #[test]
    fn xa_by_its_magic() {
        let mut xa = crate::formats::xa::MAGIC.to_vec();
        xa.extend([1, 0x01, 1, 0, 0, 0, 0, 0, 0, 0]);
        xa.resize(16 + 2304, 0);
        let m = decode(xa.clone()).expect("XA");
        assert_eq!((m.rate, m.pcm.len()), (37_800, 2 * 2016));
        // a damaged XA file is not tried as MP3
        xa.pop();
        assert!(decode(xa).is_none());
        assert!(decode(Vec::new()).is_none());
        // a .pcm file by its magic
        let mut pcm = crate::formats::pcm::MAGIC.to_vec();
        pcm.extend([1, 0x44, 0xAC, 0, 0, 2, 0, 0, 0, 1, 0, 0, 0, 0, 0, 0, 0, 7, 0, 9, 0]);
        let m = decode(pcm).expect("PCM");
        assert_eq!((m.rate, m.pcm), (44_100, vec![7, 9]));
    }
}
