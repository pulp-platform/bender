// Copyright (c) 2017-2018 ETH Zurich
// Fabian Schuiki <fschuiki@iis.ee.ethz.ch>

//! Target specifications
//!
//! This module implements the boolean expressions that allow source file groups
//! to be only compile under certain target configurations.

#![deny(missing_docs)]

use std;
use std::collections::BTreeSet;
use std::fmt;
use std::str::FromStr;

use indexmap::IndexSet;
use serde::{Deserialize, Deserializer, Serialize, Serializer};

use crate::error::{Error, Result};

/// A target specification.
#[derive(Clone, Ord, PartialOrd, Eq, PartialEq, Default, Hash)]
pub enum TargetSpec {
    /// Matches all targets.
    #[default]
    Wildcard,
    /// A target that must be present.
    Name(String),
    /// All targets must be present. This is an AND operation.
    All(BTreeSet<Self>),
    /// At least one target must be present. This is an OR operation.
    Any(BTreeSet<Self>),
    /// Negates a specification. This is a NOT operation.
    Not(Box<Self>),
}

impl fmt::Display for TargetSpec {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match *self {
            Self::Wildcard => write!(f, "*"),
            Self::Name(ref name) => write!(f, "{name}"),
            Self::All(ref specs) => write!(f, "all({})", SpecsWriter(specs.iter())),
            Self::Any(ref specs) => write!(f, "any({})", SpecsWriter(specs.iter())),
            Self::Not(ref spec) => write!(f, "not({spec})"),
        }
    }
}

struct SpecsWriter<'a, T: Iterator<Item = &'a TargetSpec> + Clone + 'a>(T);

impl<'a, T> fmt::Display for SpecsWriter<'a, T>
where
    T: Iterator<Item = &'a TargetSpec> + Clone + 'a,
{
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        use std::iter::{once, repeat};
        for (sep, val) in once("").chain(repeat(", ")).zip(self.0.clone()) {
            write!(f, "{sep}{val}")?;
        }
        Ok(())
    }
}

impl fmt::Debug for TargetSpec {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        fmt::Display::fmt(self, f)
    }
}

impl FromStr for TargetSpec {
    type Err = Error;
    fn from_str(s: &str) -> std::result::Result<Self, Error> {
        let mut iter = s.chars();
        let next = iter.next();
        let mut lexer = TargetLexer {
            inner: iter,
            partial: None,
            next,
        };
        parse(&mut lexer).map_err(|cause| {
            Error::chain(
                format!("Syntax error in target specification `{s}`."),
                cause,
            )
        })
    }
}

impl Serialize for TargetSpec {
    fn serialize<S>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        format!("{self}").serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for TargetSpec {
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        use serde::de;
        let s = String::deserialize(deserializer)?;
        Self::from_str(&s).map_err(de::Error::custom)
    }
}

impl TargetSpec {
    /// Checks whether this specification matches a set of targets.
    #[must_use]
    pub fn matches(&self, targets: &TargetSet) -> bool {
        match *self {
            Self::Wildcard => true,
            Self::Name(ref name) => targets.0.contains(name),
            Self::All(ref specs) => specs.iter().all(|s| s.matches(targets)),
            Self::Any(ref specs) => specs.iter().any(|s| s.matches(targets)),
            Self::Not(ref spec) => !spec.matches(targets),
        }
    }

    /// Check whether this specification is just a wildcard.
    #[must_use]
    pub const fn is_wildcard(&self) -> bool {
        matches!(*self, Self::Wildcard)
    }

    /// Reduce this target specification to its simplest form.
    #[must_use]
    pub fn reduce(&self) -> Self {
        match self {
            Self::Wildcard => Self::Wildcard,
            Self::Name(n) => Self::Name(n.clone()),
            Self::All(set) | Self::Any(set) => {
                let set = set
                    .iter()
                    .map(Self::reduce)
                    .filter(|s| !matches!(s, Self::Wildcard))
                    .collect::<BTreeSet<_>>();
                match set.len() {
                    0 => Self::Wildcard,
                    1 => set.iter().next().unwrap().clone(),
                    _ => {
                        if matches!(self, Self::All(_)) {
                            Self::All(set)
                        } else {
                            Self::Any(set)
                        }
                    }
                }
            }
            Self::Not(t) => Self::Not(Box::new(t.reduce())),
        }
    }

    /// Get list of available targets.
    pub fn get_avail(&self) -> IndexSet<String> {
        match *self {
            Self::Wildcard => IndexSet::new(),
            Self::Name(ref name) => IndexSet::from([name.clone()]),
            Self::All(ref specs) | Self::Any(ref specs) => {
                specs.iter().flat_map(Self::get_avail).collect()
            }
            Self::Not(ref spec) => spec.get_avail(),
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
enum TargetToken {
    Ident(String),
    LParen,
    RParen,
    Comma,
    Any,
    All,
    Not,
}

struct TargetLexer<T>
where
    T: Iterator<Item = char>,
{
    inner: T,
    partial: Option<String>,
    next: Option<char>,
}

impl<T> Iterator for TargetLexer<T>
where
    T: Iterator<Item = char>,
{
    type Item = Result<TargetToken>;
    fn next(&mut self) -> Option<Result<TargetToken>> {
        loop {
            let next_is_letter = self.next.is_some_and(|c| {
                c.is_alphanumeric() || c == '.' || c == '_' || c == '-' || c == ':'
            });

            // Flush if needed.
            if !next_is_letter {
                let mut partial = None;
                std::mem::swap(&mut self.partial, &mut partial);
                if let Some(partial) = partial {
                    if partial == "all" {
                        return Some(Ok(TargetToken::All));
                    } else if partial == "any" {
                        return Some(Ok(TargetToken::Any));
                    } else if partial == "not" {
                        return Some(Ok(TargetToken::Not));
                    }
                    return Some(Ok(TargetToken::Ident(partial)));
                }
            }

            // Aggregate if needed.
            if next_is_letter {
                if self.partial.is_none() {
                    self.partial = Some(String::new());
                }
                self.partial
                    .as_mut()
                    .unwrap()
                    .extend(self.next.unwrap().to_lowercase());
                self.next = self.inner.next();
                continue;
            }

            // Emit tokens.
            let next = self.next;
            self.next = self.inner.next();
            match next {
                Some('(') => return Some(Ok(TargetToken::LParen)),
                Some(')') => return Some(Ok(TargetToken::RParen)),
                Some(',') => return Some(Ok(TargetToken::Comma)),
                Some(c) if c.is_whitespace() => (),
                Some(c) => return Some(Err(Error::new(format!("Invalid character `{c}`.")))),
                None => return None,
            }
        }
    }
}

fn parse<T>(lexer: &mut TargetLexer<T>) -> Result<TargetSpec>
where
    T: Iterator<Item = char>,
{
    Ok(match lexer.next() {
        Some(Ok(TargetToken::All)) => TargetSpec::All(parse_paren_list(lexer)?),
        Some(Ok(TargetToken::Any)) => TargetSpec::Any(parse_paren_list(lexer)?),
        Some(Ok(TargetToken::Not)) => {
            parse_require(lexer, TargetToken::LParen, "Expected `(`.")?;
            let spec = parse(lexer)?;
            parse_require(lexer, TargetToken::RParen, "Expected `)`.")?;
            TargetSpec::Not(Box::new(spec))
        }
        Some(Ok(TargetToken::Ident(name))) => {
            if name.contains(':') {
                return Err(Error::new("Targets names cannot contain colons (`:`)."));
            }
            if name.starts_with('-') {
                return Err(Error::new("Target names cannot start with a hyphen (`-`)."));
            }
            TargetSpec::Name(name)
        }
        Some(Ok(TargetToken::LParen)) => {
            let spec = parse(lexer)?;
            parse_require(lexer, TargetToken::RParen, "Expected `)`.")?;
            spec
        }
        wrong => return parse_wrong(wrong),
    })
}

fn parse_paren_list<T>(lexer: &mut TargetLexer<T>) -> Result<BTreeSet<TargetSpec>>
where
    T: Iterator<Item = char>,
{
    parse_require(lexer, TargetToken::LParen, "Expected `(`.")?;
    let mut set = BTreeSet::new();
    set.insert(parse(lexer)?);
    loop {
        match lexer.next() {
            Some(Ok(TargetToken::RParen)) => break,
            Some(Ok(TargetToken::Comma)) => (),
            wrong => return parse_wrong(wrong),
        }
        set.insert(parse(lexer)?);
    }
    Ok(set)
}

fn parse_require<T>(lexer: &mut TargetLexer<T>, token: TargetToken, msg: &str) -> Result<()>
where
    T: Iterator<Item = char>,
{
    match lexer.next() {
        Some(Ok(ref tkn)) if tkn == &token => Ok(()),
        Some(Err(e)) => Err(e),
        _ => Err(Error::new(msg)),
    }
}

fn parse_wrong<R>(wrong: Option<Result<TargetToken>>) -> Result<R> {
    match wrong {
        Some(Ok(TargetToken::All)) => Err(Error::new("Unexpected `all` keyword.")),
        Some(Ok(TargetToken::Any)) => Err(Error::new("Unexpected `any` keyword.")),
        Some(Ok(TargetToken::Not)) => Err(Error::new("Unexpected `not` keyword.")),
        Some(Ok(TargetToken::Ident(name))) => {
            Err(Error::new(format!("Unexpected identifier `{name}`.")))
        }
        Some(Ok(TargetToken::LParen)) => Err(Error::new("Unexpected `(`.")),
        Some(Ok(TargetToken::RParen)) => Err(Error::new("Unexpected `)`.")),
        Some(Ok(TargetToken::Comma)) => Err(Error::new("Unexpected `,`.")),
        Some(Err(e)) => Err(e),
        None => Err(Error::new("Unexpected end of string.")),
    }
}

/// A set of targets.
///
/// Target specifications can be matched against a target set. A target set is
/// basically just a collection of strings.
#[derive(Clone, Debug, Serialize, Default)]
pub struct TargetSet(IndexSet<String>);

impl TargetSet {
    /// Create an empty target set.
    #[must_use]
    pub fn empty() -> Self {
        Self(Default::default())
    }

    /// Create a target set.
    ///
    /// `targets` can be anything that may be turned into an iterator over
    /// something that can be turned into a `&str`.
    pub fn new<I>(targets: I) -> Self
    where
        I: IntoIterator,
        I::Item: AsRef<str>,
    {
        let targets: IndexSet<String> = targets
            .into_iter()
            .map(|t| t.as_ref().to_lowercase())
            .collect();
        Self(targets)
    }

    /// Returns true if the set of targets is empty.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// Get an iterator over this set.
    pub fn iter(&self) -> impl Iterator<Item = &String> {
        self.0.iter()
    }

    /// Insert a target into the set.
    pub fn insert(&mut self, target: String) {
        self.0.insert(target);
    }

    /// Reduce target set for a dependency.
    #[must_use]
    pub fn reduce_for_dependency(&self, dep_name: &str) -> Self {
        // collect targets relevant to the dependency
        let local_targets = Self::new(self.iter().filter_map(|trgt| {
            if trgt.contains(':') {
                let parts: Vec<&str> = trgt.splitn(2, ':').collect();
                if dep_name == parts[0].to_lowercase().as_str() || parts[0] == "*" {
                    Some(parts[1].to_string())
                } else {
                    None
                }
            } else {
                Some(trgt.clone())
            }
        }));

        // collect negative targets to be removed
        let neg_targets = local_targets
            .iter()
            .filter_map(|t| t.strip_prefix('-').map(std::string::ToString::to_string))
            .collect::<IndexSet<_>>();

        // remove negative targets from all_targets
        Self::new(local_targets.iter().filter_map(|t| {
            if t.starts_with('-') || neg_targets.contains(t) {
                None
            } else {
                Some(t.clone())
            }
        }))
    }
}

impl<'a> IntoIterator for &'a TargetSet {
    type Item = <&'a IndexSet<String> as IntoIterator>::Item;
    type IntoIter = <&'a IndexSet<String> as IntoIterator>::IntoIter;
    fn into_iter(self) -> Self::IntoIter {
        self.0.iter()
    }
}

impl IntoIterator for TargetSet {
    type Item = <IndexSet<String> as IntoIterator>::Item;
    type IntoIter = <IndexSet<String> as IntoIterator>::IntoIter;
    fn into_iter(self) -> Self::IntoIter {
        self.0.into_iter()
    }
}
