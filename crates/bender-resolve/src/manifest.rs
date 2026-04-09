//! Minimal `Bender.yml` manifest parser for dependency extraction.
//!
//! This module provides lightweight types that deserialize only the fields of a
//! `Bender.yml` manifest needed for dependency resolution: the package name,
//! remotes, and the dependency map. All other fields (sources, plugins,
//! workspace, etc.) are silently ignored by serde.
//!
//! These types intentionally duplicate a small subset of the full manifest
//! representation in the root `bender` crate (`src/config.rs`). The full types
//! live in a binary crate and cannot be depended on from here.

use indexmap::IndexMap;
use serde::Deserialize;

/// Errors that can occur while parsing a manifest's dependencies.
#[derive(Debug, thiserror::Error)]
pub enum ManifestError {
    /// YAML deserialization failed.
    #[error("failed to parse manifest: {0}")]
    Yaml(#[from] serde_yaml_ng::Error),

    /// A version-only dependency was found but no default remote is configured.
    #[error(
        "dependency `{name}` specifies only a version but no default remote is configured \
         in the manifest"
    )]
    NoDefaultRemote { name: String },

    /// A named remote was referenced but not defined in the manifest.
    #[error("dependency `{dep}` references unknown remote `{remote}`")]
    UnknownRemote { dep: String, remote: String },

    /// Invalid combination of dependency fields.
    #[error("invalid dependency `{name}`: {reason}")]
    InvalidDependency { name: String, reason: String },

    /// The version string could not be parsed as a semver requirement.
    #[error("invalid version requirement `{version}` for dependency `{dep}`: {source}")]
    InvalidVersion {
        dep: String,
        version: String,
        source: semver::Error,
    },
}

// ---------------------------------------------------------------------------
// Deserialization types
// ---------------------------------------------------------------------------

/// Minimal manifest: only the fields needed for dependency extraction.
#[derive(Debug, Deserialize)]
pub struct PartialManifest {
    pub package: Option<Package>,
    pub remotes: Option<IndexMap<String, Remote>>,
    dependencies: Option<IndexMap<String, StringOrDep>>,
}

/// Package metadata — we only need the name.
#[derive(Debug, Deserialize)]
pub struct Package {
    pub name: String,
}

/// A remote can be specified as a bare URL string or as a struct.
#[derive(Debug, Deserialize)]
#[serde(untagged)]
pub enum Remote {
    Url(String),
    Full {
        url: String,
        #[serde(default)]
        default: bool,
    },
}

/// A dependency entry in the flat form it appears in the YAML.
///
/// Mirrors `PartialDependency` in the main bender crate. Unknown fields are
/// silently absorbed by `extra` — the same pattern used in `config.rs`.
#[derive(Debug, Default, Deserialize)]
struct RawDep {
    path: Option<String>,
    git: Option<String>,
    rev: Option<String>,
    /// Accepts unquoted YAML scalars (integers, floats) as well as strings,
    /// because real-world manifests often write `version: 0.2.4` without quotes.
    #[serde(default, deserialize_with = "de_scalar_str_opt")]
    version: Option<String>,
    remote: Option<String>,
    /// Absorbs any unrecognised fields so serde does not reject them.
    #[serde(flatten)]
    #[allow(dead_code)]
    extra: std::collections::HashMap<String, serde::de::IgnoredAny>,
}

/// Deserializes `Option<String>` from any YAML scalar (string, int, float).
fn de_scalar_str_opt<'de, D: serde::Deserializer<'de>>(d: D) -> Result<Option<String>, D::Error> {
    struct V;
    impl<'de> serde::de::Visitor<'de> for V {
        type Value = Option<String>;
        fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
            f.write_str("a string or number")
        }
        fn visit_none<E: serde::de::Error>(self) -> Result<Self::Value, E> {
            Ok(None)
        }
        fn visit_some<D: serde::Deserializer<'de>>(self, d: D) -> Result<Self::Value, D::Error> {
            d.deserialize_any(Self)
                .map(|opt| opt.or(Some(String::new())))
        }
        fn visit_str<E: serde::de::Error>(self, v: &str) -> Result<Self::Value, E> {
            Ok(Some(v.to_string()))
        }
        fn visit_string<E: serde::de::Error>(self, v: String) -> Result<Self::Value, E> {
            Ok(Some(v))
        }
        fn visit_i64<E: serde::de::Error>(self, v: i64) -> Result<Self::Value, E> {
            Ok(Some(v.to_string()))
        }
        fn visit_u64<E: serde::de::Error>(self, v: u64) -> Result<Self::Value, E> {
            Ok(Some(v.to_string()))
        }
        fn visit_f64<E: serde::de::Error>(self, v: f64) -> Result<Self::Value, E> {
            Ok(Some(v.to_string()))
        }
    }
    d.deserialize_any(V)
}

/// Deserializes a dependency value that is either a bare version string or a
/// mapping. Mirrors the `StringOrStruct` pattern from the main bender crate.
///
/// A bare string like `dep: "1.0.0"` is treated as `{version: "1.0.0"}`.
#[derive(Debug)]
struct StringOrDep(RawDep);

impl<'de> Deserialize<'de> for StringOrDep {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        struct V;
        impl<'de> serde::de::Visitor<'de> for V {
            type Value = StringOrDep;
            fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
                f.write_str("a version string or a dependency mapping")
            }
            fn visit_str<E: serde::de::Error>(self, v: &str) -> Result<Self::Value, E> {
                Ok(StringOrDep(RawDep {
                    version: Some(v.to_string()),
                    ..Default::default()
                }))
            }
            fn visit_i64<E: serde::de::Error>(self, v: i64) -> Result<Self::Value, E> {
                self.visit_str(&v.to_string())
            }
            fn visit_u64<E: serde::de::Error>(self, v: u64) -> Result<Self::Value, E> {
                self.visit_str(&v.to_string())
            }
            fn visit_f64<E: serde::de::Error>(self, v: f64) -> Result<Self::Value, E> {
                self.visit_str(&v.to_string())
            }
            fn visit_map<A: serde::de::MapAccess<'de>>(
                self,
                map: A,
            ) -> Result<Self::Value, A::Error> {
                RawDep::deserialize(serde::de::value::MapAccessDeserializer::new(map))
                    .map(StringOrDep)
            }
        }
        d.deserialize_any(V)
    }
}

// ---------------------------------------------------------------------------
// Parsed output types
// ---------------------------------------------------------------------------

/// A dependency whose kind and parameters have been determined.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ParsedDependency {
    /// Semver version requirement resolved via a git remote.
    GitVersion {
        url: String,
        version: semver::VersionReq,
    },
    /// A pinned git revision (commit, branch, or tag).
    GitRevision { url: String, rev: String },
    /// A local filesystem path.
    Path(String),
}

// ---------------------------------------------------------------------------
// Manifest logic
// ---------------------------------------------------------------------------

impl PartialManifest {
    /// Deserialize a `Bender.yml` string into a [`PartialManifest`].
    pub fn parse(yaml: &str) -> Result<Self, ManifestError> {
        Ok(serde_yaml_ng::from_str(yaml)?)
    }

    /// Resolve all dependencies, expanding remote URLs where needed.
    ///
    /// Returns `(dependency_name, parsed)` pairs.
    pub fn resolve_dependencies(&self) -> Result<Vec<(String, ParsedDependency)>, ManifestError> {
        let deps = match &self.dependencies {
            Some(d) => d,
            None => return Ok(Vec::new()),
        };

        // Pre-compute remote lookup.
        let remotes = self.remotes.as_ref();
        let default_remote = remotes.and_then(|r| {
            // A single remote is implicitly the default (matches bender's behaviour).
            if r.len() == 1 {
                return Some(r.values().next().unwrap().url());
            }
            r.values().find_map(|v| match v {
                Remote::Full {
                    url, default: true, ..
                } => Some(url.as_str()),
                _ => None,
            })
        });

        let mut result = Vec::with_capacity(deps.len());
        for (name, StringOrDep(raw)) in deps {
            let parsed = resolve_one(name, raw, remotes, default_remote)?;
            result.push((name.clone(), parsed));
        }
        Ok(result)
    }
}

impl Remote {
    fn url(&self) -> &str {
        match self {
            Remote::Url(u) => u,
            Remote::Full { url, .. } => url,
        }
    }
}

/// Build a full git URL from a remote URL template and a package name.
///
/// If the template contains `{}`, the package name is substituted in.
/// Otherwise the package name is appended as `{base_url}/{name}.git`.
fn expand_remote_url(template: &str, pkg_name: &str) -> String {
    let trimmed = template.trim();
    if trimmed.contains("{}") {
        trimmed.replace("{}", pkg_name)
    } else {
        let base = trimmed.trim_end_matches('/');
        format!("{}/{}.git", base, pkg_name)
    }
}

/// Parse a version string into a semver requirement.
fn parse_version(dep: &str, version: &str) -> Result<semver::VersionReq, ManifestError> {
    semver::VersionReq::parse(version).map_err(|e| ManifestError::InvalidVersion {
        dep: dep.to_string(),
        version: version.to_string(),
        source: e,
    })
}

/// Resolve a single dependency entry.
fn resolve_one(
    name: &str,
    raw: &RawDep,
    remotes: Option<&IndexMap<String, Remote>>,
    default_remote: Option<&str>,
) -> Result<ParsedDependency, ManifestError> {
    match (
        raw.git.as_deref(),
        raw.path.as_deref(),
        raw.rev.as_deref(),
        raw.version.as_deref(),
        raw.remote.as_deref(),
    ) {
        // Path dependency: {path: "..."}
        (None, Some(p), None, None, _) => Ok(ParsedDependency::Path(p.to_string())),

        // Version-only with default remote: `dep: "1.0.0"` or `{version: "X"}`
        (None, None, None, Some(v), None) => {
            let version = parse_version(name, v)?;
            let url_template = default_remote.ok_or_else(|| ManifestError::NoDefaultRemote {
                name: name.to_string(),
            })?;
            Ok(ParsedDependency::GitVersion {
                url: expand_remote_url(url_template, name),
                version,
            })
        }

        // Version with named remote: {version: "X", remote: "name"}
        (None, None, None, Some(v), Some(r)) => {
            let version = parse_version(name, v)?;
            let remote_cfg =
                remotes
                    .and_then(|rs| rs.get(r))
                    .ok_or_else(|| ManifestError::UnknownRemote {
                        dep: name.to_string(),
                        remote: r.to_string(),
                    })?;
            Ok(ParsedDependency::GitVersion {
                url: expand_remote_url(remote_cfg.url(), name),
                version,
            })
        }

        // Explicit git + version: {git: "url", version: "X"}
        (Some(g), None, None, Some(v), None) => {
            let version = parse_version(name, v)?;
            Ok(ParsedDependency::GitVersion {
                url: g.to_string(),
                version,
            })
        }

        // Git + revision: {git: "url", rev: "ref"}
        (Some(g), None, Some(r), None, None) => Ok(ParsedDependency::GitRevision {
            url: g.to_string(),
            rev: r.to_string(),
        }),

        // Invalid combinations
        (_, _, Some(_), Some(_), _) => Err(ManifestError::InvalidDependency {
            name: name.to_string(),
            reason: "cannot specify both `version` and `rev`".to_string(),
        }),
        (g, Some(_), r, v, _) if g.is_some() || r.is_some() || v.is_some() => {
            Err(ManifestError::InvalidDependency {
                name: name.to_string(),
                reason: "cannot combine `path` with `git`, `rev`, or `version`".to_string(),
            })
        }
        (Some(_), _, None, None, _) => Err(ManifestError::InvalidDependency {
            name: name.to_string(),
            reason: "`git` requires either `rev` or `version`".to_string(),
        }),
        (Some(_), _, _, _, Some(_)) => Err(ManifestError::InvalidDependency {
            name: name.to_string(),
            reason: "cannot combine `git` and `remote`".to_string(),
        }),
        (None, None, None, None, _) => Err(ManifestError::InvalidDependency {
            name: name.to_string(),
            reason: "must specify at least one of `path`, `git`, `rev`, or `version`".to_string(),
        }),
        cfg => Err(ManifestError::InvalidDependency {
            name: name.to_string(),
            reason: format!("invalid field combination: {cfg:?}"),
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_basic_manifest() {
        let yaml = r#"
package:
  name: MyPkg

remotes:
  pulp:
    url: "https://github.com/pulp-platform"
    default: true

dependencies:
  common_cells: "1.39.0"
  common_verification: { version: "0.2.5", remote: "pulp" }
  tech_cells_generic:
    git: "https://github.com/pulp-platform/tech_cells_generic.git"
    version: "0.2.13"
"#;
        let manifest = PartialManifest::parse(yaml).unwrap();
        assert_eq!(
            manifest.package.as_ref().unwrap().name.to_lowercase(),
            "mypkg"
        );

        let deps = manifest.resolve_dependencies().unwrap();
        assert_eq!(deps.len(), 3);

        // common_cells: version via default remote
        match &deps[0].1 {
            ParsedDependency::GitVersion { url, .. } => {
                assert_eq!(url, "https://github.com/pulp-platform/common_cells.git");
            }
            other => panic!("expected GitVersion, got {other:?}"),
        }
    }

    #[test]
    fn version_dep_single_remote_is_default() {
        let yaml = r#"
package:
  name: test

remotes:
  pulp: "https://github.com/pulp-platform"

dependencies:
  dep: "1.0.0"
"#;
        let manifest = PartialManifest::parse(yaml).unwrap();
        let deps = manifest.resolve_dependencies().unwrap();
        assert_eq!(deps.len(), 1);
        match &deps[0].1 {
            ParsedDependency::GitVersion { url, .. } => {
                assert_eq!(url, "https://github.com/pulp-platform/dep.git");
            }
            other => panic!("expected GitVersion, got {other:?}"),
        }
    }

    #[test]
    fn version_dep_needs_default_remote_with_multiple_remotes() {
        let yaml = r#"
package:
  name: test

remotes:
  pulp: "https://github.com/pulp-platform"
  other: "https://github.com/other"

dependencies:
  dep: "1.0.0"
"#;
        let manifest = PartialManifest::parse(yaml).unwrap();
        let err = manifest.resolve_dependencies().unwrap_err();
        assert!(matches!(err, ManifestError::NoDefaultRemote { .. }));
    }

    #[test]
    fn version_dep_with_default_remote() {
        let yaml = r#"
package:
  name: test

remotes:
  pulp:
    url: "https://github.com/pulp-platform/{}.git"
    default: true

dependencies:
  common_cells: ">=1.0.0"
"#;
        let manifest = PartialManifest::parse(yaml).unwrap();
        let deps = manifest.resolve_dependencies().unwrap();
        assert_eq!(deps.len(), 1);
        assert_eq!(deps[0].0, "common_cells");
        match &deps[0].1 {
            ParsedDependency::GitVersion { url, version } => {
                assert_eq!(url, "https://github.com/pulp-platform/common_cells.git");
                assert!(version.matches(&semver::Version::new(1, 5, 0)));
            }
            other => panic!("expected GitVersion, got {other:?}"),
        }
    }

    #[test]
    fn version_dep_with_named_remote() {
        let yaml = r#"
package:
  name: test

remotes:
  pulp: "https://github.com/pulp-platform"

dependencies:
  dep: { version: "0.2.5", remote: "pulp" }
"#;
        let manifest = PartialManifest::parse(yaml).unwrap();
        let deps = manifest.resolve_dependencies().unwrap();
        assert_eq!(deps.len(), 1);
        match &deps[0].1 {
            ParsedDependency::GitVersion { url, .. } => {
                assert_eq!(url, "https://github.com/pulp-platform/dep.git");
            }
            other => panic!("expected GitVersion, got {other:?}"),
        }
    }

    #[test]
    fn git_version_dep() {
        let yaml = r#"
package:
  name: test

dependencies:
  dep:
    git: "https://github.com/example/dep.git"
    version: "0.2.13"
"#;
        let manifest = PartialManifest::parse(yaml).unwrap();
        let deps = manifest.resolve_dependencies().unwrap();
        assert_eq!(deps.len(), 1);
        match &deps[0].1 {
            ParsedDependency::GitVersion { url, version } => {
                assert_eq!(url, "https://github.com/example/dep.git");
                assert!(version.matches(&semver::Version::new(0, 2, 13)));
            }
            other => panic!("expected GitVersion, got {other:?}"),
        }
    }

    #[test]
    fn git_revision_dep() {
        let yaml = r#"
package:
  name: test

dependencies:
  dep:
    git: "https://github.com/example/dep.git"
    rev: "abc1234"
"#;
        let manifest = PartialManifest::parse(yaml).unwrap();
        let deps = manifest.resolve_dependencies().unwrap();
        assert_eq!(deps.len(), 1);
        match &deps[0].1 {
            ParsedDependency::GitRevision { url, rev } => {
                assert_eq!(url, "https://github.com/example/dep.git");
                assert_eq!(rev, "abc1234");
            }
            other => panic!("expected GitRevision, got {other:?}"),
        }
    }

    #[test]
    fn path_dep() {
        let yaml = r#"
package:
  name: test

dependencies:
  local_dep: { path: "../local_dep" }
"#;
        let manifest = PartialManifest::parse(yaml).unwrap();
        let deps = manifest.resolve_dependencies().unwrap();
        assert_eq!(deps.len(), 1);
        assert_eq!(
            deps[0].1,
            ParsedDependency::Path("../local_dep".to_string())
        );
    }

    #[test]
    fn invalid_rev_and_version() {
        let yaml = r#"
package:
  name: test

dependencies:
  dep:
    git: "https://example.com/dep.git"
    rev: "abc"
    version: "1.0.0"
"#;
        let manifest = PartialManifest::parse(yaml).unwrap();
        let err = manifest.resolve_dependencies().unwrap_err();
        assert!(matches!(err, ManifestError::InvalidDependency { .. }));
    }

    #[test]
    fn invalid_path_with_git() {
        let yaml = r#"
package:
  name: test

dependencies:
  dep:
    path: "../foo"
    git: "https://example.com/dep.git"
    rev: "abc"
"#;
        let manifest = PartialManifest::parse(yaml).unwrap();
        let err = manifest.resolve_dependencies().unwrap_err();
        assert!(matches!(err, ManifestError::InvalidDependency { .. }));
    }

    #[test]
    fn unknown_remote() {
        let yaml = r#"
package:
  name: test

dependencies:
  dep: { version: "1.0.0", remote: "nonexistent" }
"#;
        let manifest = PartialManifest::parse(yaml).unwrap();
        let err = manifest.resolve_dependencies().unwrap_err();
        assert!(matches!(err, ManifestError::UnknownRemote { .. }));
    }

    #[test]
    fn no_dependencies() {
        let yaml = r#"
package:
  name: test
"#;
        let manifest = PartialManifest::parse(yaml).unwrap();
        let deps = manifest.resolve_dependencies().unwrap();
        assert!(deps.is_empty());
    }

    #[test]
    fn dep_unknown_fields_ignored() {
        let yaml = r#"
package:
  name: test

dependencies:
  dep:
    git: "https://github.com/example/dep.git"
    version: "1.0.0"
    commit: "abc123"
    some_unknown_field: true
"#;
        let manifest = PartialManifest::parse(yaml).unwrap();
        let deps = manifest.resolve_dependencies().unwrap();
        assert_eq!(deps.len(), 1);
        match &deps[0].1 {
            ParsedDependency::GitVersion { url, .. } => {
                assert_eq!(url, "https://github.com/example/dep.git");
            }
            other => panic!("expected GitVersion, got {other:?}"),
        }
    }

    #[test]
    fn dep_unquoted_prerelease_version() {
        let yaml = r#"
package:
  name: test

dependencies:
  dep:
    git: "https://github.com/example/dep.git"
    version: 0.0.0-alpha.10
"#;
        let manifest = PartialManifest::parse(yaml).unwrap();
        let deps = manifest.resolve_dependencies().unwrap();
        assert_eq!(deps.len(), 1);
        match &deps[0].1 {
            ParsedDependency::GitVersion { .. } => {}
            other => panic!("expected GitVersion, got {other:?}"),
        }
    }

    #[test]
    fn extra_fields_ignored() {
        let yaml = r#"
package:
  name: test
  authors: ["someone"]
  description: "a package"

sources:
  - file.sv

workspace:
  checkout_dir: "./deps"

dependencies:
  dep: { path: "../dep" }
"#;
        let manifest = PartialManifest::parse(yaml).unwrap();
        let deps = manifest.resolve_dependencies().unwrap();
        assert_eq!(deps.len(), 1);
    }
}
