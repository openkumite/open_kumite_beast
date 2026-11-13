//! The two optional imports' converters behind the setup's disc image readers (engine addition): the imported game's
//! disc (`br1`), read through `ok_br2_converter`'s image readers (BIN/CUE with its CD-DA track through `open_cue_all`,
//! CHD with its audio tracks, ISO) and converted by `ok_br1_converter`'s `runtime::Converter` into the game data's `br1/`
//! folder; and Quake III Arena's `.pk3` files (`q3`), read through `ReadAt` and converted by `ok_q3_converter`'s
//! `runtime::Converter` into its `q3/` folder.

/// The imported game's disc: an image opened with the main converter's readers, checked and converted.
pub mod br1 {
    pub use ok_br1_converter::Error;
    pub use ok_br1_converter::disc::Disc;
    pub use ok_br1_converter::runtime::{Converter, Sink};
    use ok_br2_converter::disc::RAW_SECTOR;
    use ok_br2_converter::{DiscError, SectorReader};

    /// An image reader of the main converter as the imported converter's `Sectors`.
    struct Sectors(Box<dyn SectorReader>);

    fn error(e: DiscError) -> Error {
        match e {
            DiscError::Io(e) => Error::Io(e),
            DiscError::Format(e) => Error::Format(e),
            DiscError::WrongDisc { found } => Error::WrongDisc { found },
            DiscError::NotRaw2352(e) => Error::NotRaw2352(e),
        }
    }

    impl ok_br1_converter::disc::Sectors for Sectors {
        fn raw(&self) -> bool {
            self.0.raw()
        }
        fn sectors(&self) -> u32 {
            self.0.sectors()
        }
        fn read_sector(&mut self, lba: u32, out: &mut [u8]) -> ok_br1_converter::Result<()> {
            self.0.read_sector(lba, out).map_err(error)
        }
        fn source(&self) -> &'static str {
            self.0.source()
        }
        fn audio_tracks(&self) -> Vec<ok_br1_converter::disc::AudioTrack> {
            self.0.audio_tracks().into_iter().map(|t| ok_br1_converter::disc::AudioTrack { number: t.number, sectors: t.sectors }).collect()
        }
        fn read_audio(&mut self, track: u32, sector: u32, out: &mut [u8]) -> ok_br1_converter::Result<()> {
            self.0.read_audio(track, sector, &mut out[..RAW_SECTOR]).map_err(error)
        }
    }

    /// The disc on `image`, checked to be the imported game's (`Error::WrongDisc` naming what it is otherwise, e.g.
    /// the main disc's volume id).
    pub fn open(image: Box<dyn SectorReader>) -> Result<Disc, Error> {
        Disc::open_checked(Box::new(Sectors(image)))
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        /// A 2048-byte-sector ISO whose primary volume descriptor (sector 16) names a PlayStation disc
        /// `volume`, its root directory (sector 17) empty.
        fn tiny_iso(volume: &str) -> Vec<u8> {
            let mut iso = vec![0u8; 2048 * 19];
            let pvd = &mut iso[16 * 2048..17 * 2048];
            pvd[0] = 1;
            pvd[1..6].copy_from_slice(b"CD001");
            pvd[6] = 1;
            pvd[8..40].copy_from_slice(format!("{:<32}", "PLAYSTATION").as_bytes());
            pvd[40..72].copy_from_slice(format!("{volume:<32}").as_bytes());
            // the root directory's record: sector 17, 2048 bytes, a directory
            let root = &mut pvd[156..190];
            root[0] = 34;
            root[2..6].copy_from_slice(&17u32.to_le_bytes());
            root[6..10].copy_from_slice(&17u32.to_be_bytes());
            root[10..14].copy_from_slice(&2048u32.to_le_bytes());
            root[14..18].copy_from_slice(&2048u32.to_be_bytes());
            root[25] = 2;
            root[32] = 1;
            iso[18 * 2048] = 255; // the volume descriptor set terminator
            iso[18 * 2048 + 1..18 * 2048 + 6].copy_from_slice(b"CD001");
            iso
        }

        #[test]
        fn the_main_disc_is_refused() {
            let main = ok_br2_converter::disc::VOLUME_ID;
            let e = open(ok_br2_converter::memory_image(tiny_iso(main)).unwrap()).err().unwrap();
            assert!(matches!(&e, Error::WrongDisc { found } if found.contains(main)), "{e}");
        }
    }
}

/// Quake III Arena's paks: opened through `ReadAt` and converted.
pub mod q3 {
    use ok_br2_converter::{ReadAt, ReadAtStream};
    pub use ok_q3_converter::runtime::{Converter, Error, Paks, Sink};

    /// The paks as (file name, reader): only their zip directories are read here (`Error::NoMaps` without a
    /// `maps/*.bsp`). Each reader is buffered: the directory walk is thousands of small reads (in a browser, each one a
    /// `FileReaderSync` call).
    pub fn open(paks: Vec<(String, Box<dyn ReadAt>)>) -> Result<Paks, Error> {
        Paks::open(
            paks.into_iter()
                .map(|(name, r)| {
                    let r: Box<dyn ok_q3_converter::vfs::ReadSeek> = Box::new(std::io::BufReader::with_capacity(256 << 10, ReadAtStream::new(r)));
                    (name, r)
                })
                .collect(),
        )
    }

    #[cfg(test)]
    mod tests {
        use super::*;
        use std::io::Write;

        struct Mem(Vec<u8>);
        impl ReadAt for Mem {
            fn read_at(&mut self, offset: u64, buf: &mut [u8]) -> std::io::Result<()> {
                let o = offset as usize;
                let src = self.0.get(o..o + buf.len()).ok_or(std::io::ErrorKind::UnexpectedEof)?;
                buf.copy_from_slice(src);
                Ok(())
            }
            fn len(&self) -> u64 {
                self.0.len() as u64
            }
        }

        fn pk3(files: &[(&str, Vec<u8>)]) -> Box<dyn ReadAt> {
            let mut z = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
            for (name, data) in files {
                z.start_file(*name, zip::write::SimpleFileOptions::default()).unwrap();
                z.write_all(data).unwrap();
            }
            Box::new(Mem(z.finish().unwrap().into_inner()))
        }

        #[test]
        fn a_tiny_pak_through_open_and_run() {
            // a map of no surfaces (an IBSP v46 header, every lump empty) and its levelshot, a 1×1 TGA
            let mut bsp = b"IBSP".to_vec();
            bsp.extend(46i32.to_le_bytes());
            bsp.resize(8 + 17 * 8, 0);
            let tga = vec![0, 0, 2, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1, 0, 1, 0, 24, 0, 30, 20, 10];
            let paks = open(vec![("pak0.pk3".into(), pk3(&[("maps/tiny.bsp", bsp), ("levelshots/tiny.tga", tga)]))]).unwrap();
            assert_eq!(paks.maps(), ["tiny"]);
            let mut sink = ok_q3_converter::runtime::MemorySink::default();
            let summary = Converter::new(paks).run(&mut sink, &mut |_| true).unwrap();
            assert_eq!(summary.maps, ["tiny"]);
            let names: Vec<&str> = sink.files.keys().map(String::as_str).collect();
            assert_eq!(names, ["baseq3/levelshots/tiny.png", "baseq3/maps/tiny.entities.json", "baseq3/maps/tiny.glb", "manifest.json", "shaders.json"]);
            let m: serde_json::Value = serde_json::from_slice(&sink.files["manifest.json"]).unwrap();
            assert_eq!(m["game"].as_str(), Some(ok_q3_converter::runtime::GAME));
            assert_eq!(m["files"].as_array().map(|f| f.len()), Some(4));
            // not a zip; a zip without maps
            assert!(matches!(open(vec![("pak0.pk3".into(), Box::new(Mem(b"not a zip".to_vec())))]), Err(Error::Format(_))));
            assert!(matches!(open(vec![("pak0.pk3".into(), pk3(&[("a.txt", vec![1])]))]), Err(Error::NoMaps)));
        }
    }
}
