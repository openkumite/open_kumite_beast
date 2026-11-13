//! Serde helpers of the state and the file formats: long arrays, byte strings.
//!
//! State: none (the encodings of the serialised state and files)
//!
//! Serde helpers of the state and file formats (`#[serde(with = "crate::base::serde_util::…")]`): long arrays, byte
//! strings. The encodings are those of `session` / `session::replay` before the move.

/// Serde for arrays longer than serde's 32 (the AI table, the chain slots …): as a sequence.
pub mod big_array {
    use serde::de::Error;
    use serde::{Deserialize, Deserializer, Serialize, Serializer};

    pub fn serialize<S: Serializer, T: Serialize, const N: usize>(a: &[T; N], s: S) -> Result<S::Ok, S::Error> {
        a.as_slice().serialize(s)
    }

    pub fn deserialize<'de, D: Deserializer<'de>, T: Deserialize<'de>, const N: usize>(d: D) -> Result<[T; N], D::Error> {
        let v = Vec::<T>::deserialize(d)?;
        let n = v.len();
        v.try_into().map_err(|_| D::Error::invalid_length(n, &"the array's length"))
    }
}

/// Serde for bytes as one CBOR byte string (not an array of numbers).
pub mod blob {
    use serde::de::{SeqAccess, Visitor};
    use serde::{Deserializer, Serializer};
    use std::fmt;

    pub fn serialize<S: Serializer>(b: &[u8], s: S) -> Result<S::Ok, S::Error> {
        s.serialize_bytes(b)
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<Vec<u8>, D::Error> {
        d.deserialize_byte_buf(Bytes)
    }

    struct Bytes;

    impl<'de> Visitor<'de> for Bytes {
        type Value = Vec<u8>;

        fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
            f.write_str("bytes")
        }
        fn visit_bytes<E: serde::de::Error>(self, b: &[u8]) -> Result<Vec<u8>, E> {
            Ok(b.to_vec())
        }
        fn visit_byte_buf<E: serde::de::Error>(self, b: Vec<u8>) -> Result<Vec<u8>, E> {
            Ok(b)
        }
        fn visit_seq<A: SeqAccess<'de>>(self, mut a: A) -> Result<Vec<u8>, A::Error> {
            let mut v = Vec::with_capacity(a.size_hint().unwrap_or(0).min(4096));
            while let Some(x) = a.next_element()? {
                v.push(x);
            }
            Ok(v)
        }
    }
}

/// `blob` for an optional byte string.
pub mod opt_blob {
    use serde::{Deserialize, Deserializer, Serialize, Serializer};

    struct Ref<'a>(&'a [u8]);

    impl Serialize for Ref<'_> {
        fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
            s.serialize_bytes(self.0)
        }
    }

    #[derive(Deserialize)]
    struct Owned(#[serde(with = "super::blob")] Vec<u8>);

    pub fn serialize<S: Serializer>(b: &Option<Vec<u8>>, s: S) -> Result<S::Ok, S::Error> {
        match b {
            Some(b) => s.serialize_some(&Ref(b)),
            None => s.serialize_none(),
        }
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<Option<Vec<u8>>, D::Error> {
        Ok(Option::<Owned>::deserialize(d)?.map(|b| b.0))
    }
}
