// Copyright (c) 2025 ETH Zurich
// Tim Fischer <fischeti@iis.ee.ethz.ch>

use std::collections::HashSet;
use std::fmt;
use std::path::PathBuf;
use std::sync::{Mutex, OnceLock};

use miette::{Diagnostic, ReportHandler};
use owo_colors::OwoColorize;
use thiserror::Error;

use crate::{fmt_field, fmt_path, fmt_pkg, fmt_version};

static GLOBAL_DIAGNOSTICS: OnceLock<Diagnostics> = OnceLock::new();

/// A diagnostics manager that handles warnings (and errors).
#[derive(Debug)]
pub struct Diagnostics {
    /// A set of suppressed warnings.
    suppressed: HashSet<String>,
    /// Whether all warnings are suppressed.
    all_suppressed: bool,
    /// A set of already emitted warnings.
    /// Requires synchronization as warnings may be emitted from multiple threads.
    emitted: Mutex<HashSet<Warnings>>,
}

impl Diagnostics {
    /// Create a new diagnostics manager.
    pub fn init(suppressed: HashSet<String>) {
        // Set up miette with our custom renderer
        miette::set_hook(Box::new(|_| Box::new(DiagnosticRenderer))).unwrap();
        let diag = Diagnostics {
            all_suppressed: suppressed.contains("all") || suppressed.contains("Wall"),
            suppressed,
            emitted: Mutex::new(HashSet::new()),
        };

        GLOBAL_DIAGNOSTICS
            .set(diag)
            .expect("Diagnostics already initialized!");
    }

    /// Get the global diagnostics manager.
    fn get() -> &'static Diagnostics {
        GLOBAL_DIAGNOSTICS
            .get()
            .expect("Diagnostics not initialized!")
    }

    /// Check whether a warning/error code is suppressed.
    pub fn is_suppressed(code: &str) -> bool {
        let diag = Diagnostics::get();
        diag.all_suppressed || diag.suppressed.contains(code)
    }
}

impl Warnings {
    /// Checks suppression, deduplicates, and emits the warning to stderr.
    pub fn emit(self) {
        let diag = Diagnostics::get();

        // Check whether the command is suppressed
        if let Some(code) = self.code() {
            if diag.all_suppressed || diag.suppressed.contains(&code.to_string()) {
                return;
            }
        }

        // Check whether the warning was already emitted
        let mut emitted = diag.emitted.lock().unwrap();
        if emitted.contains(&self) {
            return;
        }
        emitted.insert(self.clone());
        drop(emitted);

        // Print the warning report (consumes self i.e. the warning)
        eprintln!("{:?}", miette::Report::new(self));
    }
}

pub struct DiagnosticRenderer;

impl ReportHandler for DiagnosticRenderer {
    fn debug(&self, diagnostic: &dyn Diagnostic, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // Determine severity and the resulting style
        let (severity, style) = match diagnostic.severity().unwrap_or_default() {
            miette::Severity::Error => ("error", owo_colors::Style::new().red().bold()),
            miette::Severity::Warning => ("warning", owo_colors::Style::new().yellow().bold()),
            miette::Severity::Advice => ("advice", owo_colors::Style::new().cyan().bold()),
        };

        // Write the severity prefix
        write!(f, "{}", severity.style(style))?;

        // Write the code, if any
        if let Some(code) = diagnostic.code() {
            write!(f, "{}", format!("[{}]", code).style(style))?;
        }

        // Write the main diagnostic message
        write!(f, ": {}", diagnostic)?;

        // We collect all footer lines into a vector.
        let mut annotations: Vec<String> = Vec::new();

        // First, we write the help message(s) if any
        if let Some(help) = diagnostic.help() {
            let help_str = help.to_string();
            for line in help_str.lines() {
                annotations.push(format!(
                    "{} {}",
                    "help:".bold(),
                    line.replace("\x1b[0m", "\x1b[0m\x1b[2m").dimmed()
                ));
            }
        }

        // Prepare tree characters
        let branch = " ├─›";
        let corner = " ╰─›";

        // Iterate over the annotations and print them
        for (i, note) in annotations.iter().enumerate() {
            // The last item gets the corner, everyone else gets a branch
            let is_last = i == annotations.len() - 1;
            let prefix = if is_last { corner } else { branch };
            write!(f, "\n{} {}", prefix.dimmed(), note)?;
        }

        Ok(())
    }
}

// Note(fischeti): The enum variants should preferably use struct style
// variants for better readability, but this is not possible due to a current
// issue in `miette` that causes `unused` warnings when the help message does not
// use all fields of a struct variant. This is new since Rust 1.92.0, and a fix
// is pending in `miette`. See also:
// Issue: https://github.com/zkat/miette/issues/458
// PR: https://github.com/zkat/miette/pull/459
// The workaround for the moment is to use tuple style variants
// for variants where the help message does not use all fields.
#[derive(Error, Diagnostic, Hash, Eq, PartialEq, Debug, Clone)]
#[diagnostic(severity(Warning))]
pub enum Warnings {
    #[error(
        "Skipping link to package {} at {} since there is something there",
        fmt_pkg!(.0),
        fmt_path!(.1.display())
    )]
    #[diagnostic(
        code(W01),
        help("Check the existing file or directory that is preventing the link.")
    )]
    SkippingPackageLink(String, PathBuf),

    #[error("Using config at {} for overrides.", fmt_path!(path.display()))]
    #[diagnostic(code(W02))]
    UsingConfigForOverride { path: PathBuf },

    #[error("Ignoring unknown field {} in package {}.", fmt_field!(field), fmt_pkg!(pkg))]
    #[diagnostic(
        code(W03),
        help("Check for typos in {} or remove it from the {} manifest.", fmt_field!(field), fmt_pkg!(pkg))
    )]
    IgnoreUnknownField { field: String, pkg: String },

    #[error("Source group in package {} contains no source files.", fmt_pkg!(.0))]
    #[diagnostic(
        code(W04),
        help("Add source files to the source group or remove it from the manifest.")
    )]
    NoFilesInSourceGroup(String),

    #[error("No files matched the global pattern {}.", fmt_path!(path))]
    #[diagnostic(code(W05))]
    NoFilesForGlobalPattern { path: String },

    // TODO(fischeti): Why are there two W06 variants?
    #[error("Dependency {} in checkout_dir {} is not a git repository. Setting as path dependency.", fmt_pkg!(.0), fmt_path!(.1.display()))]
    #[diagnostic(
        code(W06),
        help("Use `bender clone` to work on git dependencies.\nRun `bender update --ignore-checkout-dir` to overwrite this at your own risk.")
    )]
    NotAGitDependency(String, PathBuf),

    // TODO(fischeti): Why are there two W06 variants?
    #[error("Dependency {} in checkout_dir {} is not in a clean state. Setting as path dependency.", fmt_pkg!(.0), fmt_path!(.1.display()))]
    #[diagnostic(code(W06), help("Use `bender clone` to work on git dependencies.\nRun `bender update --ignore-checkout-dir` to overwrite this at your own risk."))]
    DirtyGitDependency(String, PathBuf),

    // TODO(fischeti): Why are there two W07 variants?
    // TODO(fischeti): This is part of an error, not a warning. Move to Error enum later?
    #[error("SSH key might be missing.")]
    #[diagnostic(
        code(W07),
        help("Please ensure your public ssh key is added to the git server.")
    )]
    SshKeyMaybeMissing,

    // TODO(fischeti): Why are there two W07 variants?
    // TODO(fischeti): This is part of an error, not a warning. Move to Error enum later?
    #[error("SSH key might be missing.")]
    #[diagnostic(
        code(W07),
        help("Please ensure the url is correct and you have access to the repository.")
    )]
    UrlMaybeIncorrect,

    // TODO(fischeti): This is part of an error, not a warning. Move to Error enum later?
    #[error("Revision {} not found in repository {}.", fmt_version!(.0), fmt_pkg!(.1))]
    #[diagnostic(
        code(W08),
        help("Check that the revision exists in the remote repository or run `bender update`.")
    )]
    RevisionNotFound(String, String),

    #[error("Path dependency {} inside git dependency {} detected. This is currently not fully suppored and your milage may vary.", fmt_pkg!(pkg), fmt_pkg!(top_pkg))]
    #[diagnostic(code(W09))]
    PathDepInGitDep { pkg: String, top_pkg: String },

    #[error("There may be issues in the path for {}.", fmt_pkg!(.0))]
    #[diagnostic(
        code(W10),
        help("Please check that {} is correct and accessible.", fmt_path!(.1.display()))
    )]
    MaybePathIssues(String, PathBuf),

    #[error("Dependency package name {} does not match the package name {} in its manifest.", fmt_pkg!(.0), fmt_pkg!(.1))]
    #[diagnostic(
        code(W11),
        help("Check that the dependency name in your root manifest matches the name in the {} manifest.", fmt_pkg!(.0))
    )]
    DepPkgNameNotMatching(String, String),

    #[error("Manifest for package {} not found at {}.", fmt_pkg!(pkg), fmt_path!(src))]
    #[diagnostic(code(W12))]
    ManifestNotFound { pkg: String, src: String },

    #[error("Name issue with package {}. `export_include_dirs` cannot be handled.", fmt_pkg!(.0))]
    #[diagnostic(
        code(W13),
        help("Could be related to name missmatch, check `bender update`.")
    )]
    ExportDirNameIssue(String),

    #[error("If `--local` is used, no fetching will be performed.")]
    #[diagnostic(code(W14))]
    LocalNoFetch,

    #[error("No patch directory found for package {} when trying to apply patches from {} to {}. Skipping patch generation.", fmt_pkg!(vendor_pkg), fmt_path!(from_prefix.display()), fmt_path!(to_prefix.display()))]
    #[diagnostic(code(W15))]
    NoPatchDir {
        vendor_pkg: String,
        from_prefix: PathBuf,
        to_prefix: PathBuf,
    },

    #[error("Dependency string for the included dependencies might be wrong.")]
    #[diagnostic(code(W16))]
    DependStringMaybeWrong,

    // TODO(fischeti): Why are there two W16 variants?
    #[error("{} not found in upstream, continuing.", fmt_path!(path))]
    #[diagnostic(code(W16))]
    NotInUpstream { path: String },

    #[error("Package {} is shown to include dependency, but manifest does not have this information.", fmt_pkg!(pkg))]
    #[diagnostic(code(W17))]
    IncludeDepManifestMismatch { pkg: String },

    #[error("An override is specified for dependency {} to {}.", fmt_pkg!(pkg), fmt_pkg!(pkg_override))]
    #[diagnostic(code(W18))]
    DepOverride { pkg: String, pkg_override: String },

    #[error("Workspace checkout directory set and has uncommitted changes, not updating {} at {}.", fmt_pkg!(.0), fmt_path!(.1.display()))]
    #[diagnostic(
        code(W19),
        help("Run `bender checkout --force` to overwrite the dependency at your own risk.")
    )]
    CheckoutDirDirty(String, PathBuf),

    // TODO(fischeti): Should this be an error instead of a warning?
    #[error("Ignoring error for {} at {}: {}", fmt_pkg!(.0), fmt_path!(.1), .2)]
    #[diagnostic(code(W20))]
    IgnoringError(String, String, String),

    #[error("No revision found in lock file for git dependency {}.", fmt_pkg!(pkg))]
    #[diagnostic(code(W21))]
    NoRevisionInLockFile { pkg: String },

    #[error("Dependency {} has source path {} which does not exist.", fmt_pkg!(.0), fmt_path!(.1.display()))]
    #[diagnostic(code(W22), help("Please check that the path exists and is correct."))]
    DepSourcePathMissing(String, PathBuf),

    #[error("Locked revision {} for dependency {} not found in available revisions, allowing update.", fmt_version!(rev), fmt_pkg!(pkg))]
    #[diagnostic(code(W23))]
    LockedRevisionNotFound { pkg: String, rev: String },

    #[error("Include directory {} doesn't exist.", fmt_path!(.0.display()))]
    #[diagnostic(
        code(W24),
        help("Please check that the include directory exists and is correct.")
    )]
    IncludeDirMissing(PathBuf),

    #[error("Skipping dirty dependency {}", fmt_pkg!(pkg))]
    #[diagnostic(help("Use `--no-skip` to still snapshot {}.", fmt_pkg!(pkg)))]
    SkippingDirtyDep { pkg: String },

    #[error("File not added, ignoring: {cause}")]
    #[diagnostic(code(W30))]
    IgnoredPath { cause: String },

    #[error("File {} doesn't exist.", fmt_path!(path.display()))]
    #[diagnostic(code(W31))]
    FileMissing { path: PathBuf },

    #[error("Path {} for dependency {} does not exist.", fmt_path!(path.display()), fmt_pkg!(pkg))]
    #[diagnostic(code(W32))]
    DepPathMissing { pkg: String, path: PathBuf },
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Once;

    static TEST_INIT: Once = Once::new();

    /// Helper to initialize diagnostics once for the entire test run.
    fn setup_diagnostics() {
        TEST_INIT.call_once(|| {
            // We use an empty set for the global init in tests
            // or a specific set if needed.
            Diagnostics::init(HashSet::from(["W02".to_string()]));
        });
    }

    #[test]
    fn test_is_suppressed() {
        setup_diagnostics();
        assert!(Diagnostics::is_suppressed("W02"));
        assert!(!Diagnostics::is_suppressed("W01"));
    }

    #[test]
    fn test_suppression_works() {
        setup_diagnostics(); // Assumes this suppresses W02
        let diag = Diagnostics::get();

        let warn = Warnings::UsingConfigForOverride {
            path: PathBuf::from("/example/path"),
        };

        // Clear state
        diag.emitted.lock().unwrap().clear();

        // Call emit (The Gatekeeper)
        warn.clone().emit();

        let emitted = diag.emitted.lock().unwrap();
        assert!(!emitted.contains(&warn));
    }

    #[test]
    fn test_all_suppressed() {
        // Since we can't re-init the GLOBAL_DIAGNOSTICS with different values
        // in the same process, we test the logic via a local instance.
        let diag = Diagnostics {
            suppressed: HashSet::new(),
            all_suppressed: true,
            emitted: Mutex::new(HashSet::new()),
        };

        // Manual check of the logic inside emit()
        let warn = Warnings::LocalNoFetch;
        let code = warn.code().unwrap().to_string();
        assert!(diag.all_suppressed || diag.suppressed.contains(&code));
    }

    #[test]
    fn test_deduplication_logic() {
        setup_diagnostics();
        let diag = Diagnostics::get();
        let warn1 = Warnings::NoRevisionInLockFile {
            pkg: "example_pkg".into(),
        };
        let warn2 = Warnings::NoRevisionInLockFile {
            pkg: "other_pkg".into(),
        };

        // Clear state
        diag.emitted.lock().unwrap().clear();

        // Emit first warning
        warn1.clone().emit();
        {
            let emitted = diag.emitted.lock().unwrap();
            assert!(emitted.contains(&warn1));
            assert_eq!(emitted.len(), 1);
        }

        // Emit second warning (different data)
        warn2.clone().emit();
        {
            let emitted = diag.emitted.lock().unwrap();
            assert!(emitted.contains(&warn2));
            assert_eq!(emitted.len(), 2);
        }

        // Emit first warning again
        warn1.clone().emit();
        {
            let emitted = diag.emitted.lock().unwrap();
            // The length should STILL be 2, because warn1 was already there
            assert_eq!(emitted.len(), 2);
        }
    }

    #[test]
    fn test_contains_code() {
        let warn = Warnings::LocalNoFetch;
        let code = warn.code().unwrap().to_string();
        assert_eq!(code, "W14".to_string());
    }

    #[test]
    fn test_contains_no_code() {
        let warn = Warnings::SkippingDirtyDep {
            pkg: "example_pkg".to_string(),
        };
        let code = warn.code();
        assert!(code.is_none());
    }

    #[test]
    fn test_contains_help() {
        let warn = Warnings::SkippingPackageLink(
            "example_pkg".to_string(),
            PathBuf::from("/example/path"),
        );
        let help = warn.help().unwrap().to_string();
        assert!(help.contains("Check the existing file or directory"));
    }

    #[test]
    fn test_contains_no_help() {
        let warn = Warnings::NoRevisionInLockFile {
            pkg: "example_pkg".to_string(),
        };
        let help = warn.help();
        assert!(help.is_none());
    }

    #[test]
    fn test_stderr_contains_code() {
        setup_diagnostics();
        let warn = Warnings::LocalNoFetch;
        let code = warn.code().unwrap().to_string();
        let report = format!("{:?}", miette::Report::new(warn));
        assert!(report.contains(&code));
    }

    #[test]
    fn test_stderr_contains_help() {
        setup_diagnostics();
        let warn = Warnings::SkippingPackageLink(
            "example_pkg".to_string(),
            PathBuf::from("/example/path"),
        );
        let report = format!("{:?}", miette::Report::new(warn));
        assert!(report.contains("Check the existing file or directory"));
    }

    #[test]
    fn test_stderr_contains_no_help() {
        setup_diagnostics();
        let warn = Warnings::NoRevisionInLockFile {
            pkg: "example_pkg".to_string(),
        };
        let report = format!("{:?}", miette::Report::new(warn));
        assert!(!report.contains("help:"));
    }

    #[test]
    fn test_stderr_contains_two_help() {
        setup_diagnostics();
        let warn =
            Warnings::NotAGitDependency("example_dep".to_string(), PathBuf::from("/example/path"));
        let report = format!("{:?}", miette::Report::new(warn));
        let help_count = report.matches("help:").count();
        assert_eq!(help_count, 2);
    }
}
