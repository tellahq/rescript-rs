//! A serializer that writes every `f32` as the `f64` its shortest decimal
//! reads as, and passes everything else to the one it wraps.
//!
//! serde-wasm-bindgen widens an `f32` exactly (`0.1f32` is
//! `0.10000000149011612` in JavaScript), where JSON text of it, and the
//! `f32`'s own Spice codec, write `0.1`.

use serde::ser::{
    Serialize, SerializeMap, SerializeSeq, SerializeStruct, SerializeStructVariant, SerializeTuple,
    SerializeTupleStruct, SerializeTupleVariant, Serializer,
};

pub(crate) struct Shortest<S>(pub(crate) S);

/// A value serialized through [`Shortest`].
struct Value<'a, T: ?Sized>(&'a T);

impl<T: ?Sized + Serialize> Serialize for Value<'_, T> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        self.0.serialize(Shortest(serializer))
    }
}

macro_rules! forward {
    ($($method:ident($($arg:ident: $ty:ty),*);)*) => {$(
        fn $method(self, $($arg: $ty),*) -> Result<Self::Ok, Self::Error> {
            self.0.$method($($arg),*)
        }
    )*};
}

impl<S: Serializer> Serializer for Shortest<S> {
    type Ok = S::Ok;
    type Error = S::Error;
    type SerializeSeq = Shortest<S::SerializeSeq>;
    type SerializeTuple = Shortest<S::SerializeTuple>;
    type SerializeTupleStruct = Shortest<S::SerializeTupleStruct>;
    type SerializeTupleVariant = Shortest<S::SerializeTupleVariant>;
    type SerializeMap = Shortest<S::SerializeMap>;
    type SerializeStruct = Shortest<S::SerializeStruct>;
    type SerializeStructVariant = Shortest<S::SerializeStructVariant>;

    forward! {
        serialize_bool(v: bool);
        serialize_i8(v: i8);
        serialize_i16(v: i16);
        serialize_i32(v: i32);
        serialize_i64(v: i64);
        serialize_i128(v: i128);
        serialize_u8(v: u8);
        serialize_u16(v: u16);
        serialize_u32(v: u32);
        serialize_u64(v: u64);
        serialize_u128(v: u128);
        serialize_f64(v: f64);
        serialize_char(v: char);
        serialize_str(v: &str);
        serialize_bytes(v: &[u8]);
        serialize_none();
        serialize_unit();
        serialize_unit_struct(name: &'static str);
        serialize_unit_variant(name: &'static str, index: u32, variant: &'static str);
    }

    fn serialize_f32(self, v: f32) -> Result<Self::Ok, Self::Error> {
        self.0.serialize_f64(crate::spice::shortest(v))
    }

    fn serialize_some<T: ?Sized + Serialize>(self, value: &T) -> Result<Self::Ok, Self::Error> {
        self.0.serialize_some(&Value(value))
    }

    fn serialize_newtype_struct<T: ?Sized + Serialize>(
        self,
        name: &'static str,
        value: &T,
    ) -> Result<Self::Ok, Self::Error> {
        self.0.serialize_newtype_struct(name, &Value(value))
    }

    fn serialize_newtype_variant<T: ?Sized + Serialize>(
        self,
        name: &'static str,
        index: u32,
        variant: &'static str,
        value: &T,
    ) -> Result<Self::Ok, Self::Error> {
        self.0
            .serialize_newtype_variant(name, index, variant, &Value(value))
    }

    fn serialize_seq(self, len: Option<usize>) -> Result<Self::SerializeSeq, Self::Error> {
        self.0.serialize_seq(len).map(Shortest)
    }

    fn serialize_tuple(self, len: usize) -> Result<Self::SerializeTuple, Self::Error> {
        self.0.serialize_tuple(len).map(Shortest)
    }

    fn serialize_tuple_struct(
        self,
        name: &'static str,
        len: usize,
    ) -> Result<Self::SerializeTupleStruct, Self::Error> {
        self.0.serialize_tuple_struct(name, len).map(Shortest)
    }

    fn serialize_tuple_variant(
        self,
        name: &'static str,
        index: u32,
        variant: &'static str,
        len: usize,
    ) -> Result<Self::SerializeTupleVariant, Self::Error> {
        self.0
            .serialize_tuple_variant(name, index, variant, len)
            .map(Shortest)
    }

    fn serialize_map(self, len: Option<usize>) -> Result<Self::SerializeMap, Self::Error> {
        self.0.serialize_map(len).map(Shortest)
    }

    fn serialize_struct(
        self,
        name: &'static str,
        len: usize,
    ) -> Result<Self::SerializeStruct, Self::Error> {
        self.0.serialize_struct(name, len).map(Shortest)
    }

    fn serialize_struct_variant(
        self,
        name: &'static str,
        index: u32,
        variant: &'static str,
        len: usize,
    ) -> Result<Self::SerializeStructVariant, Self::Error> {
        self.0
            .serialize_struct_variant(name, index, variant, len)
            .map(Shortest)
    }

    fn is_human_readable(&self) -> bool {
        self.0.is_human_readable()
    }
}

macro_rules! elements {
    ($($trait:ident::$method:ident),*) => {$(
        impl<S: $trait> $trait for Shortest<S> {
            type Ok = S::Ok;
            type Error = S::Error;

            fn $method<T: ?Sized + Serialize>(&mut self, value: &T) -> Result<(), S::Error> {
                self.0.$method(&Value(value))
            }

            fn end(self) -> Result<S::Ok, S::Error> {
                self.0.end()
            }
        }
    )*};
}

elements!(
    SerializeSeq::serialize_element,
    SerializeTuple::serialize_element,
    SerializeTupleStruct::serialize_field,
    SerializeTupleVariant::serialize_field
);

impl<S: SerializeMap> SerializeMap for Shortest<S> {
    type Ok = S::Ok;
    type Error = S::Error;

    fn serialize_key<T: ?Sized + Serialize>(&mut self, key: &T) -> Result<(), S::Error> {
        self.0.serialize_key(&Value(key))
    }

    fn serialize_value<T: ?Sized + Serialize>(&mut self, value: &T) -> Result<(), S::Error> {
        self.0.serialize_value(&Value(value))
    }

    fn end(self) -> Result<S::Ok, S::Error> {
        self.0.end()
    }
}

macro_rules! fields {
    ($($trait:ident),*) => {$(
        impl<S: $trait> $trait for Shortest<S> {
            type Ok = S::Ok;
            type Error = S::Error;

            fn serialize_field<T: ?Sized + Serialize>(
                &mut self,
                key: &'static str,
                value: &T,
            ) -> Result<(), S::Error> {
                self.0.serialize_field(key, &Value(value))
            }

            fn skip_field(&mut self, key: &'static str) -> Result<(), S::Error> {
                self.0.skip_field(key)
            }

            fn end(self) -> Result<S::Ok, S::Error> {
                self.0.end()
            }
        }
    )*};
}

fields!(SerializeStruct, SerializeStructVariant);
