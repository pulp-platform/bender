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
use serde::de::{Deserialize, Deserializer};
use serde::ser::{Serialize, Serializer};

use crate::error::*;

/// A target specification.
#[derive(Clone, Ord, PartialOrd, Eq, PartialEq, Default)]
pub enum TargetSpec {
    /// Matches all targets.
    #[default]
    Wildcard,
    /// A target that must be present.
    Name(String),
    /// All targets must be present. This is an AND operation.
    All(BTreeSet<TargetSpec>),
    /// At least one target must be present. This is an OR operation.
    Any(BTreeSet<TargetSpec>),
    /// Negates a specification. This is a NOT operation.
    Not(Box<TargetSpec>),
}

impl fmt::Display for TargetSpec {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match *self {
            TargetSpec::Wildcard => write!(f, "*"),
            TargetSpec::Name(ref name) => write!(f, "{}", name),
            TargetSpec::All(ref specs) => write!(f, "all({})", SpecsWriter(specs.iter())),
            TargetSpec::Any(ref specs) => write!(f, "any({})", SpecsWriter(specs.iter())),
            TargetSpec::Not(ref spec) => write!(f, "not({})", spec),
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
            write!(f, "{}{}", sep, val)?;
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
                format!("Syntax error in target specification `{}`.", s),
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
        format!("{}", self).serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for TargetSpec {
    fn deserialize<D>(deserializer: D) -> std::result::Result<TargetSpec, D::Error>
    where
        D: Deserializer<'de>,
    {
        use serde::de;
        let s = String::deserialize(deserializer)?;
        TargetSpec::from_str(&s).map_err(de::Error::custom)
    }
}

impl TargetSpec {
    /// Checks whether this specification matches a set of targets.
    pub fn matches(&self, targets: &TargetSet) -> bool {
        match *self {
            TargetSpec::Wildcard => true,
            TargetSpec::Name(ref name) => targets.0.contains(name),
            TargetSpec::All(ref specs) => specs.iter().all(|s| s.matches(targets)),
            TargetSpec::Any(ref specs) => specs.iter().any(|s| s.matches(targets)),
            TargetSpec::Not(ref spec) => !spec.matches(targets),
        }
    }

    /// Check whether this specification is just a wildcard.
    pub fn is_wildcard(&self) -> bool {
        matches!(*self, TargetSpec::Wildcard)
    }

    /// Reduce this target specification to its simplest form.
    pub fn reduce(&self) -> Self {
        match self {
            TargetSpec::Wildcard => Self::Wildcard,
            TargetSpec::Name(n) => Self::Name(n.clone()),
            TargetSpec::All(set) | TargetSpec::Any(set) => {
                let set = set
                    .iter()
                    .map(|s| s.reduce())
                    .filter(|s| !matches!(s, Self::Wildcard))
                    .collect::<BTreeSet<_>>();
                match set.len() {
                    0 => Self::Wildcard,
                    1 => set.iter().next().unwrap().clone(),
                    _ => {
                        if matches!(self, TargetSpec::All(_)) {
                            Self::All(set)
                        } else {
                            Self::Any(set)
                        }
                    }
                }
            }
            TargetSpec::Not(t) => Self::Not(Box::new(t.reduce())),
        }
    }

    /// Get list of available targets.
    pub fn get_avail(&self) -> IndexSet<String> {
        match *self {
            TargetSpec::Wildcard => IndexSet::new(),
            TargetSpec::Name(ref name) => IndexSet::from([name.clone()]),
            TargetSpec::All(ref specs) | TargetSpec::Any(ref specs) => {
                specs.iter().flat_map(TargetSpec::get_avail).collect()
            }
            TargetSpec::Not(ref spec) => spec.get_avail(),
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
            let next_is_letter = self
                .next
                .map(|c| c.is_alphanumeric() || c == '.' || c == '_' || c == '-')
                .unwrap_or(false);

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
                    } else {
                        return Some(Ok(TargetToken::Ident(partial)));
                    }
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
                Some(c) => return Some(Err(Error::new(format!("Invalid character `{}`.", c)))),
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
        Some(Ok(TargetToken::Ident(name))) => TargetSpec::Name(name),
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
            Err(Error::new(format!("Unexpected identifier `{}`.", name)))
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
    pub fn empty() -> TargetSet {
        TargetSet(Default::default())
    }

    /// Create a target set.
    ///
    /// `targets` can be anything that may be turned into an iterator over
    /// something that can be turned into a `&str`.
    pub fn new<I>(targets: I) -> TargetSet
    where
        I: IntoIterator,
        I::Item: AsRef<str>,
    {
        let targets: IndexSet<String> = targets
            .into_iter()
            .map(|t| t.as_ref().to_lowercase())
            .collect();
        TargetSet(targets)
    }

    /// Returns true if the set of targets is empty.
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// Get an iterator over this set.
    pub fn iter(&self) -> impl Iterator<Item = &String> {
        self.0.iter()
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
