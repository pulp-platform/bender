// Copyright (c) 2017-2018 ETH Zurich
// Fabian Schuiki <fschuiki@iis.ee.ethz.ch>

//! Various utilities.

#![deny(missing_docs)]

use std;
use std::fmt;
use std::fs::File;
use std::io::prelude::*;
use std::marker::PhantomData;
use std::path::Path;
use std::str::FromStr;
use std::time::SystemTime;

use serde::de::{Deserialize, Deserializer};
use serde::ser::{Serialize, Serializer};

/// A type that cannot be materialized.
#[derive(Debug)]
pub enum Void {}

/// Create a human-readable list of the form `a, b, and c`.
pub fn string_list<I, T>(mut iter: I, sep: &str, con: &str) -> Option<String>
where
    I: Iterator<Item = T>,
    T: AsRef<str>,
{
    let mut buffer = match iter.next() {
        Some(i) => String::from(i.as_ref()),
        None => return None,
    };
    let mut last = match iter.next() {
        Some(i) => i,
        None => return Some(buffer),
    };
    let mut had_separator = false;
    for i in iter {
        buffer.push_str(sep);
        buffer.push(' ');
        buffer.push_str(last.as_ref());
        last = i;
        had_separator = true;
    }
    if had_separator {
        buffer.push_str(sep);
    }
    buffer.push(' ');
    buffer.push_str(con);
    buffer.push(' ');
    buffer.push_str(last.as_ref());
    Some(buffer)
}

/// A magic wrapper for deserializable types that also implement `FromStr`.
///
/// Allows `T` to be deserialized from a string by calling `T::from_str`. Falls
/// back to the regular deserialization if anything else is encountered.
/// Serializes the same way `T` serializes.
#[derive(Debug)]
pub struct StringOrStruct<T>(pub T);

impl<T> Serialize for StringOrStruct<T>
where
    T: Serialize,
{
    fn serialize<S>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        self.0.serialize(serializer)
    }
}

impl<'de, T> Deserialize<'de> for StringOrStruct<T>
where
    T: Deserialize<'de> + FromStr<Err = Void>,
{
    fn deserialize<D>(deserializer: D) -> std::result::Result<StringOrStruct<T>, D::Error>
    where
        D: Deserializer<'de>,
    {
        use serde::de;
        struct Visitor<T>(PhantomData<T>);

        impl<'de, T> de::Visitor<'de> for Visitor<T>
        where
            T: Deserialize<'de> + FromStr<Err = Void>,
        {
            type Value = T;

            fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
                formatter.write_str("string or map")
            }

            fn visit_str<E>(self, value: &str) -> std::result::Result<T, E>
            where
                E: de::Error,
            {
                Ok(T::from_str(value).unwrap())
            }

            fn visit_map<M>(self, visitor: M) -> std::result::Result<T, M::Error>
            where
                M: de::MapAccess<'de>,
            {
                T::deserialize(de::value::MapAccessDeserializer::new(visitor))
            }
        }

        deserializer
            .deserialize_any(Visitor::<T>(PhantomData))
            .map(|v| StringOrStruct(v))
    }
}

/// A magic wrapper for deserializable types that also implement `From<Vec<F>>`.
///
/// Allows `T` to be deserialized from an array by calling `T::from`. Falls back
/// to the regular deserialization if anything else is encountered. Serializes
/// the same way `T` serializes.
#[derive(Debug)]
pub struct SeqOrStruct<T, F>(pub T, PhantomData<F>);

impl<T, F> SeqOrStruct<T, F> {
    /// Method for creating new SeqOrStruct to keep PhantomData private
    pub fn new(item: T) -> Self {
        SeqOrStruct(item, PhantomData)
    }
}

impl<T, F> Serialize for SeqOrStruct<T, F>
where
    T: Serialize,
{
    fn serialize<S>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        self.0.serialize(serializer)
    }
}

impl<'de, T, F> Deserialize<'de> for SeqOrStruct<T, F>
where
    T: Deserialize<'de> + From<Vec<F>>,
    F: Deserialize<'de>,
{
    fn deserialize<D>(deserializer: D) -> std::result::Result<SeqOrStruct<T, F>, D::Error>
    where
        D: Deserializer<'de>,
    {
        use serde::de;
        struct Visitor<T, F>(PhantomData<T>, PhantomData<F>);

        impl<'de, T, F> de::Visitor<'de> for Visitor<T, F>
        where
            T: Deserialize<'de> + From<Vec<F>>,
            F: Deserialize<'de>,
        {
            type Value = T;

            fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
                formatter.write_str("sequence or map")
            }

            fn visit_seq<A>(self, visitor: A) -> std::result::Result<T, A::Error>
            where
                A: de::SeqAccess<'de>,
            {
                let v: Vec<F> = Vec::deserialize(de::value::SeqAccessDeserializer::new(visitor))?;
                Ok(T::from(v))
            }

            fn visit_map<M>(self, visitor: M) -> std::result::Result<T, M::Error>
            where
                M: de::MapAccess<'de>,
            {
                T::deserialize(de::value::MapAccessDeserializer::new(visitor))
            }
        }

        deserializer
            .deserialize_any(Visitor::<T, F>(PhantomData, PhantomData))
            .map(|v| SeqOrStruct(v, PhantomData))
    }
}

/// Read an entire file into a string.
pub fn read_file(path: &Path) -> std::io::Result<String> {
    let mut file = File::open(path)?;
    let mut contents = String::new();
    file.read_to_string(&mut contents)?;
    Ok(contents)
}

/// Write an entire string to a file.
pub fn write_file(path: &Path, contents: &str) -> std::io::Result<()> {
    let mut file = File::create(path)?;
    file.write_all(contents.as_bytes())?;
    Ok(())
}

/// Try to get the metadata for a file.
///
/// In case the current OS does not support the operation, or any kind of file
/// error occurs, `None` is returned.
pub fn try_modification_time<P: AsRef<Path>>(path: P) -> Option<SystemTime> {
    use std::fs::metadata;
    let md = match metadata(path) {
        Ok(md) => md,
        Err(_) => return None,
    };
    md.modified().ok()
}
