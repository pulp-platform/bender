// Copyright (c) 2026 ETH Zurich

//! Editing of the `Bender.local` file.
//!
//! Commands such as `bender clone` and `bender snapshot` add dependency
//! overrides to `Bender.local`. The file is edited with a lossless YAML
//! parser, such that comments, formatting, and unrelated entries are kept.

#![deny(missing_docs)]

use std::path::Path;
use std::str::FromStr;

use miette::{Context as _, IntoDiagnostic as _};
use yaml_edit::{Mapping, YamlFile, YamlNode};

use crate::Result;
use crate::bail;

/// A dependency override to be written to `Bender.local`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LocalOverride<'a> {
    /// Override with a local path.
    Path(&'a str),
    /// Override with a git repository at a fixed revision.
    Git {
        /// The URL of the git repository.
        url: &'a str,
        /// The revision to check out.
        rev: &'a str,
    },
}

impl LocalOverride<'_> {
    fn to_mapping(self) -> Mapping {
        let value = Mapping::new_flow();
        match self {
            LocalOverride::Path(path) => value.set("path", path),
            LocalOverride::Git { url, rev } => {
                value.set("git", url);
                value.set("rev", rev);
            }
        }
        value
    }
}

/// Add or replace dependency overrides in the `Bender.local` file at `path`.
///
/// The file and its `overrides` section are created if they do not exist.
/// Existing overrides for the same dependencies are replaced in place.
pub fn update_local_overrides(path: &Path, overrides: &[(&str, LocalOverride)]) -> Result<()> {
    let contents = if path.exists() {
        std::fs::read_to_string(path)
            .into_diagnostic()
            .wrap_err_with(|| format!("Reading Bender.local failed at {:?}.", path))?
    } else {
        String::new()
    };
    let contents = apply_local_overrides(&contents, overrides)
        .wrap_err_with(|| format!("Updating Bender.local failed at {:?}.", path))?;
    std::fs::write(path, contents)
        .into_diagnostic()
        .wrap_err_with(|| format!("Writing Bender.local failed at {:?}.", path))
}

/// Add or replace dependency overrides in the contents of a `Bender.local` file.
fn apply_local_overrides(contents: &str, overrides: &[(&str, LocalOverride)]) -> Result<String> {
    let file = YamlFile::from_str(contents)
        .into_diagnostic()
        .wrap_err("Syntax error in Bender.local.")?;
    let doc = file.ensure_document();
    let root = match doc.as_mapping() {
        Some(root) => root,
        None if doc.as_scalar().is_some_and(|s| s.is_null()) => {
            doc.set("overrides", Mapping::new());
            doc.as_mapping().unwrap()
        }
        None => bail!("Expected Bender.local to contain a mapping."),
    };

    let section = match root.get("overrides") {
        Some(YamlNode::Mapping(section)) => section,
        Some(YamlNode::Scalar(s)) if s.is_null() => {
            root.set("overrides", Mapping::new());
            root.get_mapping("overrides").unwrap()
        }
        Some(_) => bail!("Expected `overrides` in Bender.local to be a mapping."),
        None => {
            root.set("overrides", Mapping::new());
            root.get_mapping("overrides").unwrap()
        }
    };

    for (name, value) in overrides {
        let value = value.to_mapping();
        match section.find_entry_index_by_key(*name) {
            // Remove and re-insert the entry such that stale trailing comments
            // of the replaced entry are dropped.
            Some(index) => {
                section.remove(*name);
                section.insert_at_index(index, *name, &value);
            }
            None => section.set(*name, &value),
        }
    }

    Ok(file.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    use indexmap::IndexMap;

    use crate::config::PartialDependency;

    const CLONE: (&str, LocalOverride) = (
        "common_cells",
        LocalOverride::Path("working_dir/common_cells"),
    );

    /// Apply the overrides and check that the result is a valid `Bender.local`.
    fn apply(contents: &str, overrides: &[(&str, LocalOverride)]) -> String {
        let out = apply_local_overrides(contents, overrides).unwrap();
        let parsed: IndexMap<String, serde_yaml_ng::Value> = serde_yaml_ng::from_str(&out).unwrap();
        let _: IndexMap<String, PartialDependency> =
            serde_yaml_ng::from_value(parsed["overrides"].clone()).unwrap();
        out
    }

    #[test]
    fn empty_file() {
        assert_eq!(
            apply("", &[CLONE]),
            "overrides:\n  common_cells: {path: \"working_dir/common_cells\"}\n"
        );
    }

    #[test]
    fn empty_overrides() {
        assert_eq!(
            apply("overrides:\n", &[CLONE]),
            "overrides:\n  common_cells: {path: \"working_dir/common_cells\"}\n"
        );
    }

    #[test]
    fn missing_overrides() {
        assert_eq!(
            apply("# local config\nplugins:\n  foo: { path: x }\n", &[CLONE]),
            "# local config\nplugins:\n  foo: { path: x }\noverrides:\n  common_cells: {path: \"working_dir/common_cells\"}\n"
        );
    }

    #[test]
    fn replace_flow_entry() {
        assert_eq!(
            apply(
                "overrides:\n  # keep me\n  common_cells: { git: \"https://x/cc.git\", version: 1.2.3 } # pinned\n  axi: { path: ../axi }\n",
                &[CLONE]
            ),
            "overrides:\n  # keep me\n  common_cells: {path: \"working_dir/common_cells\"}\n  axi: { path: ../axi }\n"
        );
    }

    #[test]
    fn replace_block_entry() {
        assert_eq!(
            apply(
                "overrides:\n  common_cells:\n    git: https://x/cc.git\n    rev: abc\n  axi: { path: ../axi }\n",
                &[CLONE]
            ),
            "overrides:\n  common_cells: {path: \"working_dir/common_cells\"}\n  axi: { path: ../axi }\n"
        );
    }

    #[test]
    fn similar_names_untouched() {
        assert_eq!(
            apply(
                "overrides:\n  common_cells_extra: { path: ../common_cells }\n",
                &[CLONE]
            ),
            "overrides:\n  common_cells_extra: { path: ../common_cells }\n  common_cells: {path: \"working_dir/common_cells\"}\n"
        );
    }

    #[test]
    fn flow_overrides() {
        assert_eq!(
            apply("overrides: { axi: { path: ../axi } }\n", &[CLONE]),
            "overrides: { axi: { path: ../axi } , common_cells: {path: \"working_dir/common_cells\"}}\n"
        );
    }

    #[test]
    fn keep_indentation() {
        assert_eq!(
            apply("overrides:\n    axi:\n        path: ../axi\n", &[CLONE]),
            "overrides:\n    axi:\n        path: ../axi\n    common_cells: {path: \"working_dir/common_cells\"}\n"
        );
    }

    #[test]
    fn missing_trailing_newline() {
        assert_eq!(
            apply("overrides:\n  axi: { path: ../axi }", &[CLONE]),
            "overrides:\n  axi: { path: ../axi }\n  common_cells: {path: \"working_dir/common_cells\"}\n"
        );
    }

    #[test]
    fn multiple_git_overrides() {
        let overrides = [
            (
                "common_cells",
                LocalOverride::Git {
                    url: "https://x/cc.git",
                    rev: "abc",
                },
            ),
            (
                "axi",
                LocalOverride::Git {
                    url: "https://x/axi.git",
                    rev: "def",
                },
            ),
        ];
        assert_eq!(
            apply(
                "overrides:\n  common_cells: { path: \"working_dir/common_cells\" }\n  axi: { path: \"working_dir/axi\" }\n",
                &overrides
            ),
            "overrides:\n  common_cells: {git: \"https://x/cc.git\", rev: \"abc\"}\n  axi: {git: \"https://x/axi.git\", rev: \"def\"}\n"
        );
    }

    #[test]
    fn numeric_rev_stays_string() {
        let out = apply(
            "",
            &[(
                "axi",
                LocalOverride::Git {
                    url: "https://x/axi.git",
                    rev: "1234567",
                },
            )],
        );
        let parsed: serde_yaml_ng::Value = serde_yaml_ng::from_str(&out).unwrap();
        assert_eq!(
            parsed["overrides"]["axi"]["rev"],
            serde_yaml_ng::Value::from("1234567")
        );
    }

    #[test]
    fn invalid_yaml() {
        assert!(apply_local_overrides("overrides:\n  axi: { path: ../axi\n", &[CLONE]).is_err());
    }

    #[test]
    fn invalid_overrides() {
        assert!(apply_local_overrides("overrides: foo\n", &[CLONE]).is_err());
        assert!(apply_local_overrides("- foo\n", &[CLONE]).is_err());
    }
}
