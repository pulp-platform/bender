// Copyright (c) 2017-2018 ETH Zurich
// Fabian Schuiki <fschuiki@iis.ee.ethz.ch>

//! Package manifest and configuration files.
//!
//! This module provides reading and writing of package manifests and
//! configuration files.

#![deny(missing_docs)]

use std;
use std::fmt;
use std::str::FromStr;
use std::hash::Hash;
use std::collections::HashMap;
use std::marker::PhantomData;
use serde::de::{Deserialize, Deserializer};
use serde::ser::{Serialize, Serializer};
use semver::VersionReq;
use error::*;

/// A package manifest.
///
/// This is usually called `Landa.yml` in the root directory of the package.
#[derive(Debug)]
pub struct Manifest {
    /// The package definition.
    pub package: Package,
    /// The dependencies.
    pub dependencies: HashMap<String, Dependency>,
}

/// A package definition.
///
/// Contains the metadata for an individual package.
#[derive(Serialize, Deserialize, Debug)]
pub struct Package {
    /// The name of the package.
    pub name: String,
    /// A list of package authors. Each author should be of the form `John Doe
    /// <john@doe.com>`.
    pub authors: Option<Vec<String>>,
}

/// A dependency.
///
/// The name of the dependency is given implicitly by the key in the hash map
/// that this `Dependency` is accessible through.
#[derive(Debug)]
pub enum Dependency {
    /// A dependency that can be found in one of the package repositories.
    Version(VersionReq),
    /// A local path dependency. The exact version of the dependency found at
    /// the given path will be used, regardless of any actual versioning
    /// constraints.
    Path(String),
    /// A git dependency specified by a revision.
    GitRevision(String, String),
    /// A git dependency specified by a version requirement. Works similarly to
    /// the `GitRevision`, but extracts all tags of the form `v.*` from the
    /// repository and matches the version against that.
    GitVersion(String, VersionReq),
}

/// Converts partial configuration into a validated full configuration.
pub trait Validate {
    /// The output type produced by validation.
    type Output;
    /// The error type produced by validation.
    type Error;
    /// Validate self and convert into the non-partial version.
    fn validate(self) -> std::result::Result<Self::Output, Self::Error>;
}

impl<K,V> Validate for HashMap<K,V> where K: Hash + Eq, V: Validate<Error=Error> {
    type Output = HashMap<K, V::Output>;
    type Error = (K,Error);
    fn validate(self) -> std::result::Result<Self::Output, Self::Error> {
        self.into_iter().map(|(k,v)| match v.validate() {
            Ok(v) => Ok((k,v)),
            Err(e) => Err((k,e)),
        }).collect()
    }
}

/// A partial manifest.
///
/// Validation turns this into a `Manifest`.
#[derive(Serialize, Deserialize, Debug)]
pub struct PartialManifest {
    /// The package definition.
    pub package: Option<Package>,
    /// The dependencies.
    pub dependencies: HashMap<String, StringOrStruct<PartialDependency>>,
}

impl Validate for PartialManifest {
    type Output = Manifest;
    type Error = Error;
    fn validate(self) -> Result<Manifest> {
        let pkg = match self.package {
            Some(p) => p,
            None => return Err(Error::new("Missing package information."))
        };
        let deps = self.dependencies.validate().map_err(|(key,cause)| Error::chain(
            format!("In dependency `{}` of package `{}`:", key, pkg.name),
            cause
        ))?;
        Ok(Manifest {
            package: pkg,
            dependencies: deps,
        })
    }
}

/// A partial dependency.
///
/// Contains all the necessary information to resolve and find a dependency.
/// The following combinations of fields are valid:
///
/// - `version`
/// - `path`
/// - `git,rev`
/// - `git,version`
///
/// Can be validated into a `Dependency`.
#[derive(Serialize, Deserialize, Debug)]
pub struct PartialDependency {
    /// The path to the package.
    path: Option<String>,
    /// The git URL to the package.
    git: Option<String>,
    /// The git revision of the package to use. Can be a commit hash, branch,
    /// tag, or similar.
    rev: Option<String>,
    /// The version requirement of the package. This will be parsed into a
    /// semantic versioning requirement.
    version: Option<String>,
}

/// A type that never realizes.
#[derive(Debug)]
pub enum Void {}

impl FromStr for PartialDependency {
    type Err = Void;
    fn from_str(s: &str) -> std::result::Result<Self, Void> {
        Ok(PartialDependency {
            path: None,
            git: None,
            rev: None,
            version: Some(s.into()),
        })
    }
}

impl Validate for PartialDependency {
    type Output = Dependency;
    type Error = Error;
    fn validate(self) -> Result<Dependency> {
        let version = match self.version {
            Some(v) => Some(VersionReq::parse(&v).map_err(|cause| Error::chain(
                format!("\"{}\" is not a valid semantic version requirement.", v),
                cause
            ))?),
            None => None,
        };
        if self.rev.is_some() && version.is_some() {
            return Err(Error::new("A dependency cannot specify `version` and `rev` at the same time."));
        }
        if let Some(path) = self.path {
            if let Some(list) = string_list(
                self.git.map(|_| "`git`").iter()
                .chain(self.rev.map(|_| "`rev`").iter())
                .chain(version.map(|_| "`version`").iter()),
                ",", "or"
            ) {
                Err(Error::new(format!("A `path` dependency cannot have a {} field.", list)))
            } else {
                Ok(Dependency::Path(path))
            }
        } else if let Some(git) = self.git {
            if let Some(rev) = self.rev {
                Ok(Dependency::GitRevision(git, rev))
            } else if let Some(version) = version {
                Ok(Dependency::GitVersion(git, version))
            } else {
                Err(Error::new("A `git` dependency must have either a `rev` or `version` field."))
            }
        } else if let Some(version) = version {
            Ok(Dependency::Version(version))
        } else {
            Err(Error::new("A dependency must specify `version`, `path`, or `git`."))
        }
    }
}

/// Create a human-readable list of the form `a, b, and c`.
pub fn string_list<I,T>(mut iter: I, sep: &str, con: &str) -> Option<String>
    where I: Iterator<Item=T>, T: AsRef<str>
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
    while let Some(i) = iter.next() {
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
pub struct StringOrStruct<T>(T);

impl<T> Serialize for StringOrStruct<T> where T: Serialize {
    fn serialize<S>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error>
        where S: Serializer
    {
        self.0.serialize(serializer)
    }
}

impl<'de, T> Deserialize<'de> for StringOrStruct<T>
    where T: Deserialize<'de> + FromStr<Err=Void>
{
    fn deserialize<D>(deserializer: D) -> std::result::Result<StringOrStruct<T>, D::Error>
        where D: Deserializer<'de>
    {
        use serde::de;
        struct Visitor<T>(PhantomData<T>);

        impl<'de, T> de::Visitor<'de> for Visitor<T>
            where T: Deserialize<'de> + FromStr<Err=Void>
        {
            type Value = T;

            fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
                formatter.write_str("string or map")
            }

            fn visit_str<E>(self, value: &str) -> std::result::Result<T, E>
                where E: de::Error
            {
                Ok(T::from_str(value).unwrap())
            }

            fn visit_map<M>(self, visitor: M) -> std::result::Result<T, M::Error>
                where M: de::MapAccess<'de>
            {
                T::deserialize(de::value::MapAccessDeserializer::new(visitor))
            }
        }

        deserializer.deserialize_any(Visitor::<T>(PhantomData)).map(|v| StringOrStruct(v))
    }
}

impl<T> Validate for StringOrStruct<T> where T: Validate {
    type Output = T::Output;
    type Error = T::Error;
    fn validate(self) -> std::result::Result<T::Output, T::Error> {
        self.0.validate()
    }
}
