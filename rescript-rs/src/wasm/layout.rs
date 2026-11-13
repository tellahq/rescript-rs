//! A serializer that writes a value as ReScript holds it, around
//! serde-wasm-bindgen's.
//!
//! Every `f32` is written as the `f64` its shortest decimal reads as:
//! serde-wasm-bindgen widens it exactly (`0.1f32` is `0.10000000149011612`
//! in JavaScript), where JSON text of it, and the `f32`'s own Spice codec,
//! write `0.1`.
//!
//! `None` is written as `undefined`, ReScript's own `None`, where the value
//! reaches ReScript as it is; where a Spice decoder reads it first, it stays
//! the `null` JSON has.

use serde::ser::{
    Serialize, SerializeMap, SerializeSeq, SerializeStruct, SerializeStructVariant, SerializeTuple,
    SerializeTupleStruct, SerializeTupleVariant, Serializer,
};
use wasm_bindgen::JsValue;

pub(crate) struct Layout<S> {
    pub(crate) inner: S,
    /// Write `None` as `undefined`. Only serde-wasm-bindgen's serializer
    /// passes a JavaScript value through, so only it can be wrapped with
    /// this set.
    pub(crate) undefined: bool,
}

/// A value serialized through [`Layout`].
struct Value<'a, T: ?Sized> {
    value: &'a T,
    undefined: bool,
}

impl<T: ?Sized + Serialize> Serialize for Value<'_, T> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        self.value.serialize(Layout {
            inner: serializer,
            undefined: self.undefined,
        })
    }
}

impl<S> Layout<S> {
    fn value<'a, T: ?Sized>(&self, value: &'a T) -> Value<'a, T> {
        Value {
            value,
            undefined: self.undefined,
        }
    }
}

macro_rules! forward {
    ($($method:ident($($arg:ident: $ty:ty),*);)*) => {$(
        fn $method(self, $($arg: $ty),*) -> Result<Self::Ok, Self::Error> {
            self.inner.$method($($arg),*)
        }
    )*};
}

impl<S: Serializer> Serializer for Layout<S> {
    type Ok = S::Ok;
    type Error = S::Error;
    type SerializeSeq = Layout<S::SerializeSeq>;
    type SerializeTuple = Layout<S::SerializeTuple>;
    type SerializeTupleStruct = Layout<S::SerializeTupleStruct>;
    type SerializeTupleVariant = Layout<S::SerializeTupleVariant>;
    type SerializeMap = Layout<S::SerializeMap>;
    type SerializeStruct = Layout<S::SerializeStruct>;
    type SerializeStructVariant = Layout<S::SerializeStructVariant>;

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
        serialize_unit();
        serialize_unit_struct(name: &'static str);
        serialize_unit_variant(name: &'static str, index: u32, variant: &'static str);
    }

    fn serialize_f32(self, v: f32) -> Result<Self::Ok, Self::Error> {
        self.inner.serialize_f64(crate::spice::shortest(v))
    }

    fn serialize_none(self) -> Result<Self::Ok, Self::Error> {
        match self.undefined {
            true => serde_wasm_bindgen::preserve::serialize(&JsValue::UNDEFINED, self.inner),
            false => self.inner.serialize_none(),
        }
    }

    fn serialize_some<T: ?Sized + Serialize>(self, value: &T) -> Result<Self::Ok, Self::Error> {
        self.inner.serialize_some(&Value {
            value,
            undefined: self.undefined,
        })
    }

    fn serialize_newtype_struct<T: ?Sized + Serialize>(
        self,
        name: &'static str,
        value: &T,
    ) -> Result<Self::Ok, Self::Error> {
        self.inner.serialize_newtype_struct(
            name,
            &Value {
                value,
                undefined: self.undefined,
            },
        )
    }

    fn serialize_newtype_variant<T: ?Sized + Serialize>(
        self,
        name: &'static str,
        index: u32,
        variant: &'static str,
        value: &T,
    ) -> Result<Self::Ok, Self::Error> {
        self.inner.serialize_newtype_variant(
            name,
            index,
            variant,
            &Value {
                value,
                undefined: self.undefined,
            },
        )
    }

    fn serialize_seq(self, len: Option<usize>) -> Result<Self::SerializeSeq, Self::Error> {
        let undefined = self.undefined;

        self.inner
            .serialize_seq(len)
            .map(|inner| Layout { inner, undefined })
    }

    fn serialize_tuple(self, len: usize) -> Result<Self::SerializeTuple, Self::Error> {
        let undefined = self.undefined;

        self.inner
            .serialize_tuple(len)
            .map(|inner| Layout { inner, undefined })
    }

    fn serialize_tuple_struct(
        self,
        name: &'static str,
        len: usize,
    ) -> Result<Self::SerializeTupleStruct, Self::Error> {
        let undefined = self.undefined;

        self.inner
            .serialize_tuple_struct(name, len)
            .map(|inner| Layout { inner, undefined })
    }

    fn serialize_tuple_variant(
        self,
        name: &'static str,
        index: u32,
        variant: &'static str,
        len: usize,
    ) -> Result<Self::SerializeTupleVariant, Self::Error> {
        let undefined = self.undefined;

        self.inner
            .serialize_tuple_variant(name, index, variant, len)
            .map(|inner| Layout { inner, undefined })
    }

    fn serialize_map(self, len: Option<usize>) -> Result<Self::SerializeMap, Self::Error> {
        let undefined = self.undefined;

        self.inner
            .serialize_map(len)
            .map(|inner| Layout { inner, undefined })
    }

    fn serialize_struct(
        self,
        name: &'static str,
        len: usize,
    ) -> Result<Self::SerializeStruct, Self::Error> {
        let undefined = self.undefined;

        self.inner
            .serialize_struct(name, len)
            .map(|inner| Layout { inner, undefined })
    }

    fn serialize_struct_variant(
        self,
        name: &'static str,
        index: u32,
        variant: &'static str,
        len: usize,
    ) -> Result<Self::SerializeStructVariant, Self::Error> {
        let undefined = self.undefined;

        self.inner
            .serialize_struct_variant(name, index, variant, len)
            .map(|inner| Layout { inner, undefined })
    }

    fn is_human_readable(&self) -> bool {
        self.inner.is_human_readable()
    }
}

macro_rules! elements {
    ($($trait:ident::$method:ident),*) => {$(
        impl<S: $trait> $trait for Layout<S> {
            type Ok = S::Ok;
            type Error = S::Error;

            fn $method<T: ?Sized + Serialize>(&mut self, value: &T) -> Result<(), S::Error> {
                let value = self.value(value);

                self.inner.$method(&value)
            }

            fn end(self) -> Result<S::Ok, S::Error> {
                self.inner.end()
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

impl<S: SerializeMap> SerializeMap for Layout<S> {
    type Ok = S::Ok;
    type Error = S::Error;

    fn serialize_key<T: ?Sized + Serialize>(&mut self, key: &T) -> Result<(), S::Error> {
        let key = self.value(key);

        self.inner.serialize_key(&key)
    }

    fn serialize_value<T: ?Sized + Serialize>(&mut self, value: &T) -> Result<(), S::Error> {
        let value = self.value(value);

        self.inner.serialize_value(&value)
    }

    fn end(self) -> Result<S::Ok, S::Error> {
        self.inner.end()
    }
}

macro_rules! fields {
    ($($trait:ident),*) => {$(
        impl<S: $trait> $trait for Layout<S> {
            type Ok = S::Ok;
            type Error = S::Error;

            fn serialize_field<T: ?Sized + Serialize>(
                &mut self,
                key: &'static str,
                value: &T,
            ) -> Result<(), S::Error> {
                let value = self.value(value);

                self.inner.serialize_field(key, &value)
            }

            fn skip_field(&mut self, key: &'static str) -> Result<(), S::Error> {
                self.inner.skip_field(key)
            }

            fn end(self) -> Result<S::Ok, S::Error> {
                self.inner.end()
            }
        }
    )*};
}

fields!(SerializeStruct, SerializeStructVariant);
