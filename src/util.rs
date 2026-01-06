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

use semver::{Version, VersionReq};
use serde::de::{Deserialize, Deserializer};
use serde::ser::{Serialize, Serializer};

/// Re-export owo_colors for use in macros.
pub use owo_colors::OwoColorize;

use crate::error::*;

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

/// Extract excluded top bound from a version requirement.
pub fn version_req_top_bound(req: &VersionReq) -> Result<Option<Version>> {
    let mut top_bound = Version::new(u64::MAX, u64::MAX, u64::MAX);
    let mut found = false; // major, minor, patch
    for comp in req.comparators.iter() {
        match comp.op {
            semver::Op::Exact | semver::Op::LessEq => {
                let max_exact = Version {
                    major: if comp.minor.is_some() {
                        comp.major
                    } else {
                        comp.major + 1
                    },
                    minor: if comp.minor.is_some() {
                        if comp.patch.is_some() {
                            comp.minor.unwrap()
                        } else {
                            comp.minor.unwrap() + 1
                        }
                    } else {
                        0
                    },
                    patch: if comp.patch.is_some() {
                        comp.patch.unwrap() + 1
                    } else {
                        0
                    },
                    pre: semver::Prerelease::EMPTY,
                    build: semver::BuildMetadata::EMPTY,
                };
                if top_bound > max_exact {
                    found = true;
                    top_bound = max_exact;
                }
            }
            semver::Op::Greater | semver::Op::GreaterEq => {} // No upper bound
            semver::Op::Less => {
                // found = true;
                let max_less = Version {
                    major: comp.major,
                    minor: comp.minor.unwrap_or(0),
                    patch: comp.patch.unwrap_or(0),
                    pre: semver::Prerelease::EMPTY,
                    build: semver::BuildMetadata::EMPTY,
                };
                if top_bound > max_less {
                    found = true;
                    top_bound = max_less;
                }
            }
            semver::Op::Tilde => {
                let max_tilde = Version {
                    major: if comp.minor.is_some() {
                        comp.major
                    } else {
                        comp.major + 1
                    },
                    minor: if comp.minor.is_some() {
                        comp.minor.unwrap() + 1
                    } else {
                        0
                    },
                    patch: 0,
                    pre: semver::Prerelease::EMPTY,
                    build: semver::BuildMetadata::EMPTY,
                };
                if top_bound > max_tilde {
                    found = true;
                    top_bound = max_tilde;
                }
            }
            semver::Op::Caret => {
                let max_caret = if comp.major > 0 || comp.minor.is_none() {
                    Version {
                        major: comp.major + 1,
                        minor: 0,
                        patch: 0,
                        pre: semver::Prerelease::EMPTY,
                        build: semver::BuildMetadata::EMPTY,
                    }
                } else if comp.minor.unwrap() > 0 || comp.patch.is_none() {
                    Version {
                        major: comp.major,
                        minor: comp.minor.unwrap() + 1,
                        patch: 0,
                        pre: semver::Prerelease::EMPTY,
                        build: semver::BuildMetadata::EMPTY,
                    }
                } else {
                    Version {
                        major: comp.major,
                        minor: comp.minor.unwrap(),
                        patch: comp.patch.unwrap() + 1,
                        pre: semver::Prerelease::EMPTY,
                        build: semver::BuildMetadata::EMPTY,
                    }
                };
                if top_bound > max_caret {
                    found = true;
                    top_bound = max_caret;
                }
            }
            semver::Op::Wildcard => {
                let max_wildcard = Version {
                    major: if comp.minor.is_some() {
                        comp.major
                    } else {
                        comp.major + 1
                    },
                    minor: if comp.minor.is_some() {
                        comp.minor.unwrap() + 1
                    } else {
                        0
                    },
                    patch: 0,
                    pre: semver::Prerelease::EMPTY,
                    build: semver::BuildMetadata::EMPTY,
                };
                if top_bound > max_wildcard {
                    found = true;
                    top_bound = max_wildcard;
                }
            }
            _ => {
                return Err(Error::new(format!(
                    "Cannot extract top bound from version requirement: {}",
                    req
                )));
            }
        }
    }

    if found {
        Ok(Some(top_bound))
    } else {
        Ok(None)
    }
}

/// Extract bottom bound from a version requirement.
pub fn version_req_bottom_bound(req: &VersionReq) -> Result<Option<Version>> {
    let mut bottom_bound = Version::new(0, 0, 0);
    let mut found = false;
    for comp in req.comparators.iter() {
        match comp.op {
            semver::Op::Exact
            | semver::Op::GreaterEq
            | semver::Op::Tilde
            | semver::Op::Caret
            | semver::Op::Wildcard => {
                let min_exact = Version {
                    major: comp.major,
                    minor: comp.minor.unwrap_or(0),
                    patch: comp.patch.unwrap_or(0),
                    pre: comp.pre.clone(),
                    build: semver::BuildMetadata::EMPTY,
                };
                if bottom_bound < min_exact {
                    found = true;
                    bottom_bound = min_exact;
                }
            }
            semver::Op::Greater => {
                let min_greater = Version {
                    major: if comp.minor.is_some() {
                        comp.major
                    } else {
                        comp.major + 1
                    },
                    minor: if comp.minor.is_some() {
                        if comp.patch.is_some() {
                            comp.minor.unwrap() + 1
                        } else {
                            comp.minor.unwrap()
                        }
                    } else {
                        0
                    },
                    patch: if comp.patch.is_some() {
                        comp.patch.unwrap() + 1
                    } else {
                        0
                    },
                    pre: comp.pre.clone(),
                    build: semver::BuildMetadata::EMPTY,
                };
                if bottom_bound < min_greater {
                    found = true;
                    bottom_bound = min_greater;
                }
            }
            semver::Op::Less | semver::Op::LessEq => {
                // No lower bound
            }
            _ => {
                return Err(Error::new(format!(
                    "Cannot extract bottom bound from version requirement: {}",
                    req
                )));
            }
        }
    }

    if found {
        Ok(Some(bottom_bound))
    } else {
        Ok(None)
    }
}

/// Format time duration with proper units.
pub fn fmt_duration(duration: std::time::Duration) -> String {
    match duration.as_millis() {
        t if t < 1000 => format!("in {}ms", t),
        t if t < 60_000 => format!("in {:.1}s", t as f64 / 1000.0),
        t => format!("in {:.1}min", t as f64 / 60000.0),
    }
}

/// Format for `package` names in diagnostic messages.
#[macro_export]
macro_rules! fmt_pkg {
    ($pkg:expr) => {
        $crate::util::OwoColorize::bold(&$pkg)
    };
}

/// Format for `path` and `url` fields in diagnostic messages.
#[macro_export]
macro_rules! fmt_path {
    ($pkg:expr) => {
        $crate::util::OwoColorize::underline(&$pkg)
    };
}

/// Format for `field` names in diagnostic messages.
#[macro_export]
macro_rules! fmt_field {
    ($field:expr) => {
        $crate::util::OwoColorize::italic(&$field)
    };
}

/// Format for `version` and `revision` fields in diagnostic messages.
#[macro_export]
macro_rules! fmt_version {
    ($ver:expr) => {
        $crate::util::OwoColorize::bold(&$ver)
    };
}
