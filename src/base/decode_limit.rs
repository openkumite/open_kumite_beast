//! A serde deserializer wrapper that bounds the memory an untrusted state may decode into.
//!
//! A serde `Deserializer` wrapper for states from outside (a save file, a peer): every sequence element, map
//! entry and string / byte length is charged against a budget (the memory the decoded value may take), so a
//! crafted length prefix or a run of one-byte elements that decode into large values (`None` of a big enum)
//! cannot make the decoder allocate without bound. Postcard and CBOR trust their length prefixes; this does not.

use serde::de::{self, DeserializeSeed, Deserializer, EnumAccess, MapAccess, SeqAccess, VariantAccess, Visitor};
use std::cell::Cell;
use std::fmt;

/// Bytes of decoded value left.
pub struct Budget(Cell<usize>);

impl Budget {
    pub fn new(bytes: usize) -> Self {
        Self(Cell::new(bytes))
    }

    fn charge<E: de::Error>(&self, n: usize) -> Result<(), E> {
        let left = self.0.get();
        if n > left {
            return Err(E::custom("the state is larger than allowed"));
        }
        self.0.set(left - n);
        Ok(())
    }
}

/// `d` with every collection charged to `b`.
pub struct Limited<'b, D> {
    pub d: D,
    pub b: &'b Budget,
}

/// Most elements a sequence's size hint may preallocate.
const HINT: usize = 4096;

macro_rules! forward {
    ($($m:ident),*) => {$(
        fn $m<V: Visitor<'de>>(self, v: V) -> Result<V::Value, D::Error> {
            self.d.$m(Wrap { v, b: self.b })
        }
    )*};
}

impl<'de, D: Deserializer<'de>> Deserializer<'de> for Limited<'_, D> {
    type Error = D::Error;

    forward!(
        deserialize_any,
        deserialize_bool,
        deserialize_i8,
        deserialize_i16,
        deserialize_i32,
        deserialize_i64,
        deserialize_i128,
        deserialize_u8,
        deserialize_u16,
        deserialize_u32,
        deserialize_u64,
        deserialize_u128,
        deserialize_f32,
        deserialize_f64,
        deserialize_char,
        deserialize_str,
        deserialize_string,
        deserialize_bytes,
        deserialize_byte_buf,
        deserialize_option,
        deserialize_unit,
        deserialize_seq,
        deserialize_map,
        deserialize_identifier,
        deserialize_ignored_any
    );

    fn deserialize_unit_struct<V: Visitor<'de>>(self, name: &'static str, v: V) -> Result<V::Value, D::Error> {
        self.d.deserialize_unit_struct(name, Wrap { v, b: self.b })
    }
    fn deserialize_newtype_struct<V: Visitor<'de>>(self, name: &'static str, v: V) -> Result<V::Value, D::Error> {
        self.d.deserialize_newtype_struct(name, Wrap { v, b: self.b })
    }
    fn deserialize_tuple<V: Visitor<'de>>(self, len: usize, v: V) -> Result<V::Value, D::Error> {
        self.d.deserialize_tuple(len, Wrap { v, b: self.b })
    }
    fn deserialize_tuple_struct<V: Visitor<'de>>(self, name: &'static str, len: usize, v: V) -> Result<V::Value, D::Error> {
        self.d.deserialize_tuple_struct(name, len, Wrap { v, b: self.b })
    }
    fn deserialize_struct<V: Visitor<'de>>(self, name: &'static str, fields: &'static [&'static str], v: V) -> Result<V::Value, D::Error> {
        self.d.deserialize_struct(name, fields, Wrap { v, b: self.b })
    }
    fn deserialize_enum<V: Visitor<'de>>(self, name: &'static str, variants: &'static [&'static str], v: V) -> Result<V::Value, D::Error> {
        self.d.deserialize_enum(name, variants, Wrap { v, b: self.b })
    }
    fn is_human_readable(&self) -> bool {
        self.d.is_human_readable()
    }
}

/// A visitor whose nested deserializers and accesses are `Limited`.
struct Wrap<'b, V> {
    v: V,
    b: &'b Budget,
}

macro_rules! visit {
    ($($m:ident: $t:ty),*) => {$(
        fn $m<E: de::Error>(self, x: $t) -> Result<V::Value, E> {
            self.v.$m(x)
        }
    )*};
}

impl<'de, V: Visitor<'de>> Visitor<'de> for Wrap<'_, V> {
    type Value = V::Value;

    fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
        self.v.expecting(f)
    }

    visit!(visit_bool: bool, visit_i8: i8, visit_i16: i16, visit_i32: i32, visit_i64: i64, visit_i128: i128, visit_u8: u8, visit_u16: u16,
        visit_u32: u32, visit_u64: u64, visit_u128: u128, visit_f32: f32, visit_f64: f64, visit_char: char);

    fn visit_str<E: de::Error>(self, x: &str) -> Result<V::Value, E> {
        self.b.charge(x.len())?;
        self.v.visit_str(x)
    }
    fn visit_borrowed_str<E: de::Error>(self, x: &'de str) -> Result<V::Value, E> {
        self.b.charge(x.len())?;
        self.v.visit_borrowed_str(x)
    }
    fn visit_string<E: de::Error>(self, x: String) -> Result<V::Value, E> {
        self.b.charge(x.len())?;
        self.v.visit_string(x)
    }
    fn visit_bytes<E: de::Error>(self, x: &[u8]) -> Result<V::Value, E> {
        self.b.charge(x.len())?;
        self.v.visit_bytes(x)
    }
    fn visit_borrowed_bytes<E: de::Error>(self, x: &'de [u8]) -> Result<V::Value, E> {
        self.b.charge(x.len())?;
        self.v.visit_borrowed_bytes(x)
    }
    fn visit_byte_buf<E: de::Error>(self, x: Vec<u8>) -> Result<V::Value, E> {
        self.b.charge(x.len())?;
        self.v.visit_byte_buf(x)
    }
    fn visit_none<E: de::Error>(self) -> Result<V::Value, E> {
        self.v.visit_none()
    }
    fn visit_unit<E: de::Error>(self) -> Result<V::Value, E> {
        self.v.visit_unit()
    }
    fn visit_some<D: Deserializer<'de>>(self, d: D) -> Result<V::Value, D::Error> {
        self.v.visit_some(Limited { d, b: self.b })
    }
    fn visit_newtype_struct<D: Deserializer<'de>>(self, d: D) -> Result<V::Value, D::Error> {
        self.v.visit_newtype_struct(Limited { d, b: self.b })
    }
    fn visit_seq<A: SeqAccess<'de>>(self, a: A) -> Result<V::Value, A::Error> {
        self.v.visit_seq(Wrap { v: a, b: self.b })
    }
    fn visit_map<A: MapAccess<'de>>(self, a: A) -> Result<V::Value, A::Error> {
        self.v.visit_map(Wrap { v: a, b: self.b })
    }
    fn visit_enum<A: EnumAccess<'de>>(self, a: A) -> Result<V::Value, A::Error> {
        self.v.visit_enum(Wrap { v: a, b: self.b })
    }
}

/// An element seed whose deserializer is `Limited`.
struct Seed<'b, S> {
    s: S,
    b: &'b Budget,
}

impl<'de, S: DeserializeSeed<'de>> DeserializeSeed<'de> for Seed<'_, S> {
    type Value = S::Value;
    fn deserialize<D: Deserializer<'de>>(self, d: D) -> Result<S::Value, D::Error> {
        self.s.deserialize(Limited { d, b: self.b })
    }
}

impl<'de, A: SeqAccess<'de>> SeqAccess<'de> for Wrap<'_, A> {
    type Error = A::Error;
    fn next_element_seed<T: DeserializeSeed<'de>>(&mut self, s: T) -> Result<Option<T::Value>, A::Error> {
        self.b.charge(std::mem::size_of::<T::Value>().max(1))?;
        self.v.next_element_seed(Seed { s, b: self.b })
    }
    fn size_hint(&self) -> Option<usize> {
        self.v.size_hint().map(|n| n.min(HINT))
    }
}

impl<'de, A: MapAccess<'de>> MapAccess<'de> for Wrap<'_, A> {
    type Error = A::Error;
    fn next_key_seed<K: DeserializeSeed<'de>>(&mut self, s: K) -> Result<Option<K::Value>, A::Error> {
        self.b.charge(std::mem::size_of::<K::Value>().max(1))?;
        self.v.next_key_seed(Seed { s, b: self.b })
    }
    fn next_value_seed<T: DeserializeSeed<'de>>(&mut self, s: T) -> Result<T::Value, A::Error> {
        self.b.charge(std::mem::size_of::<T::Value>())?;
        self.v.next_value_seed(Seed { s, b: self.b })
    }
    fn size_hint(&self) -> Option<usize> {
        self.v.size_hint().map(|n| n.min(HINT))
    }
}

impl<'de, 'b, A: EnumAccess<'de>> EnumAccess<'de> for Wrap<'b, A> {
    type Error = A::Error;
    type Variant = Wrap<'b, A::Variant>;
    fn variant_seed<T: DeserializeSeed<'de>>(self, s: T) -> Result<(T::Value, Self::Variant), A::Error> {
        let (v, a) = self.v.variant_seed(Seed { s, b: self.b })?;
        Ok((v, Wrap { v: a, b: self.b }))
    }
}

impl<'de, A: VariantAccess<'de>> VariantAccess<'de> for Wrap<'_, A> {
    type Error = A::Error;
    fn unit_variant(self) -> Result<(), A::Error> {
        self.v.unit_variant()
    }
    fn newtype_variant_seed<T: DeserializeSeed<'de>>(self, s: T) -> Result<T::Value, A::Error> {
        self.v.newtype_variant_seed(Seed { s, b: self.b })
    }
    fn tuple_variant<V: Visitor<'de>>(self, len: usize, v: V) -> Result<V::Value, A::Error> {
        self.v.tuple_variant(len, Wrap { v, b: self.b })
    }
    fn struct_variant<V: Visitor<'de>>(self, fields: &'static [&'static str], v: V) -> Result<V::Value, A::Error> {
        self.v.struct_variant(fields, Wrap { v, b: self.b })
    }
}

/// `T` decoded through `Limited` with a budget of `MAX` bytes: works with any format's entry point (`postcard::
/// from_bytes`, `ciborium::from_reader`), whose own deserializer types stay hidden.
pub struct Budgeted<T, const MAX: usize>(pub T);

impl<'de, T: serde::Deserialize<'de>, const MAX: usize> serde::Deserialize<'de> for Budgeted<T, MAX> {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let b = Budget::new(MAX);
        T::deserialize(Limited { d, b: &b }).map(Budgeted)
    }
}
