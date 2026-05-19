// Copyright (c) 2025 ETH Zurich
// Tim Fischer <fischeti@iis.ee.ethz.ch>

use std::marker::PhantomData;

use cxx::{SharedPtr, UniquePtr};
use thiserror::Error;

pub use ffi::SlangPrintOpts;

pub type Result<T> = std::result::Result<T, SlangError>;

#[derive(Debug, Error)]
pub enum SlangError {
    #[error("Failed to parse source group: {message}")]
    ParseGroup { message: String },
    #[error("Failed to trim files by top modules: {message}")]
    TrimByTop { message: String },
    #[error("Failed to access parsed syntax tree: {message}")]
    TreeAccess { message: String },
    #[error("Failed to rewrite syntax trees: {message}")]
    Rewrite { message: String },
    #[error("Failed to walk design: {message}")]
    Walk { message: String },
}

#[derive(Debug, Clone, Copy, Default)]
pub struct RenameStats {
    pub renamed_declarations: u64,
    pub renamed_references: u64,
}

#[cxx::bridge]
mod ffi {
    /// Options for the syntax printer
    #[derive(Clone, Copy)]
    struct SlangPrintOpts {
        expand_macros: bool,
        include_directives: bool,
        include_comments: bool,
        squash_newlines: bool,
    }

    /// A simple ordered key/value pair shared with C++. Used for parameter and
    /// port bindings on instantiations to preserve declaration order.
    #[derive(Clone)]
    struct KgKeyValue {
        key: String,
        value: String,
    }

    /// Knowledge-graph parameter record.
    #[derive(Clone)]
    struct KgParam {
        name: String,
        kind: String,
        default_value: String,
        is_type_param: bool,
    }

    /// Knowledge-graph port record.
    #[derive(Clone)]
    struct KgPort {
        name: String,
        direction: String,
        type_str: String,
        width_expr: String,
        bit_width: i64,
        is_type_param: bool,
    }

    /// Knowledge-graph instantiation record.
    #[derive(Clone)]
    struct KgInstance {
        module_name: String,
        instance_name: String,
        param_bindings: Vec<KgKeyValue>,
        port_bindings: Vec<KgKeyValue>,
        line_start: i64,
        line_end: i64,
    }

    /// Knowledge-graph package import record.
    #[derive(Clone)]
    struct KgImport {
        package_name: String,
        is_wildcard: bool,
        specific_symbols: Vec<String>,
    }

    /// Knowledge-graph module record.
    #[derive(Clone)]
    struct KgModule {
        name: String,
        file_path: String,
        is_package: bool,
        is_interface: bool,
        line_start: i64,
        line_end: i64,
        param_block_start: i64,
        param_block_end: i64,
        port_block_start: i64,
        port_block_end: i64,
        parameters: Vec<KgParam>,
        ports: Vec<KgPort>,
        instantiations: Vec<KgInstance>,
        imports: Vec<KgImport>,
    }

    /// Result of `walk_design`.
    #[derive(Clone)]
    struct KgWalkResult {
        modules: Vec<KgModule>,
        warnings: Vec<String>,
    }

    /// Resolved bit width for a single port. `total` is the type's
    /// `getBitWidth()`; `fields` carries a dot-flattened breakdown across
    /// nested packed structs / unions and is empty for scalar ports.
    ///
    /// `element_count > 0` indicates the canonical type is a packed array
    /// (`req_t [N-1:0]`), in which case `element_total` and `element_fields`
    /// describe one array element's layout (recursively flattened). Cxx
    /// bridge types can't easily nest, so the array template is kept flat
    /// here and folded into the typed model on the Rust side.
    #[derive(Clone)]
    struct KgPortWidth {
        name: String,
        total: i64,
        fields: Vec<KgKeyValue>,
        element_count: i64,
        element_total: i64,
        element_fields: Vec<KgKeyValue>,
    }

    /// Per-instance context produced by the elaborated walk: resolved
    /// parameter values and port widths in the parent module's frame.
    #[derive(Clone)]
    struct KgInstanceContext {
        parent_module: String,
        instance_name: String,
        child_module: String,
        param_bindings: Vec<KgKeyValue>,
        port_widths: Vec<KgPortWidth>,
    }

    /// Result of `walk_elaborated`.
    #[derive(Clone)]
    struct KgElabResult {
        contexts: Vec<KgInstanceContext>,
        warnings: Vec<String>,
    }

    unsafe extern "C++" {
        include!("bender-slang/cpp/slang_bridge.h");
        include!("slang/syntax/SyntaxTree.h");

        /// Opaque session that owns parse contexts and syntax trees.
        type SlangSession;

        /// Opaque type for the Slang syntax tree.
        #[namespace = "slang::syntax"]
        type SyntaxTree;
        type SyntaxTreeRewriter;

        fn new_slang_session() -> UniquePtr<SlangSession>;

        /// Toggle cross-group macro propagation. When enabled (before any
        /// `parse_group` call), `\`define`s declared in earlier groups are
        /// inherited by later groups, mirroring vcs / `vlog -mfcu` semantics.
        fn set_single_unit(self: Pin<&mut SlangSession>, enable: bool);

        /// Toggle lenient parsing. When enabled, parse-time error
        /// diagnostics are still reported but do NOT abort the build; the
        /// indexer ingests whichever modules survived parsing. Useful for
        /// repos with encrypted vendor IP, missing `\`include`s, or other
        /// hostile inputs that still admit a partial graph.
        fn set_lenient(self: Pin<&mut SlangSession>, enable: bool);

        fn parse_group(
            self: Pin<&mut SlangSession>,
            files: &Vec<String>,
            includes: &Vec<String>,
            defines: &Vec<String>,
        ) -> Result<()>;

        fn reachable_tree_indices(session: &SlangSession, tops: &Vec<String>) -> Result<Vec<u32>>;

        /// Keep only the trees at the given indices (in-place); drop the rest.
        /// Used by the kg pipeline to prune to the set of trees reachable from
        /// a top module before the downstream walks. Subsequent `walk_design`
        /// / `walk_elaborated` calls operate on the pruned set automatically.
        fn retain_trees(self: Pin<&mut SlangSession>, indices: &Vec<u32>);

        fn tree_count(session: &SlangSession) -> usize;

        fn tree_at(session: &SlangSession, index: usize) -> Result<SharedPtr<SyntaxTree>>;

        fn new_syntax_tree_rewriter() -> UniquePtr<SyntaxTreeRewriter>;
        fn set_suffix(self: Pin<&mut SyntaxTreeRewriter>, suffix: &str);
        fn set_excludes(self: Pin<&mut SyntaxTreeRewriter>, excludes: Vec<String>);
        fn rewrite_declarations(
            self: Pin<&mut SyntaxTreeRewriter>,
            tree: SharedPtr<SyntaxTree>,
        ) -> SharedPtr<SyntaxTree>;
        fn set_prefix(self: Pin<&mut SyntaxTreeRewriter>, prefix: &str);
        fn rewrite_references(
            self: Pin<&mut SyntaxTreeRewriter>,
            tree: SharedPtr<SyntaxTree>,
        ) -> SharedPtr<SyntaxTree>;
        fn renamed_declarations(rewriter: &SyntaxTreeRewriter) -> u64;
        fn renamed_references(rewriter: &SyntaxTreeRewriter) -> u64;

        fn print_tree(tree: SharedPtr<SyntaxTree>, options: SlangPrintOpts) -> String;

        fn dump_tree_json(tree: SharedPtr<SyntaxTree>) -> String;

        /// Walk the parsed design, extracting structured records suitable for
        /// building a knowledge graph. Returns one [`KgModule`] per declared
        /// module/interface/package across all parsed source groups.
        fn walk_design(session: &SlangSession) -> Result<KgWalkResult>;

        /// Build a slang `Compilation` from the session's parsed trees, force
        /// elaboration from the requested top modules, and harvest resolved
        /// per-instance parameter bindings and port widths.
        fn walk_elaborated(session: &SlangSession, tops: &Vec<String>) -> Result<KgElabResult>;
    }
}

pub use ffi::{
    KgElabResult, KgImport, KgInstance, KgInstanceContext, KgKeyValue, KgModule, KgParam, KgPort,
    KgPortWidth, KgWalkResult,
};

/// Public owner for all parsed trees and parse contexts.
pub struct SlangSession {
    inner: UniquePtr<ffi::SlangSession>,
}

// SAFETY: the underlying C++ `SlangSession` owns all of its state through
// either `std::vector` or `std::shared_ptr` and is never aliased across
// threads internally. We use it to run `walk_elaborated` on a worker
// thread while the main thread drives Grafeo writes, so the only
// thread-related operation is moving the unique-pointer wrapper between
// threads — that's safe so long as both threads observe a happens-before
// boundary (the `std::thread::Builder::spawn` / scope-thread join calls
// already provide one).
unsafe impl Send for SlangSession {}

/// Borrowed syntax-tree handle tied to the owning session lifetime.
pub struct SyntaxTree<'a> {
    inner: SharedPtr<ffi::SyntaxTree>,
    _session: PhantomData<&'a SlangSession>,
}

pub struct SyntaxTreeRewriter {
    inner: UniquePtr<ffi::SyntaxTreeRewriter>,
}

impl<'a> Clone for SyntaxTree<'a> {
    fn clone(&self) -> Self {
        Self {
            inner: self.inner.clone(),
            _session: PhantomData,
        }
    }
}

impl<'a> SyntaxTree<'a> {
    /// Displays the syntax tree as a string with the given options.
    pub fn display(&self, options: SlangPrintOpts) -> String {
        ffi::print_tree(self.inner.clone(), options)
    }

    /// Dumps the syntax tree as JSON for debugging purposes.
    pub fn as_debug(&self) -> String {
        ffi::dump_tree_json(self.inner.clone())
    }
}

impl std::fmt::Display for SyntaxTree<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let options = SlangPrintOpts {
            expand_macros: false,
            include_directives: true,
            include_comments: true,
            squash_newlines: false,
        };
        f.write_str(&self.display(options))
    }
}

impl std::fmt::Debug for SyntaxTree<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.as_debug())
    }
}

impl SlangSession {
    pub fn new() -> Self {
        Self {
            inner: ffi::new_slang_session(),
        }
    }

    /// Toggle cross-group macro propagation. Call before any `parse_group`
    /// call to enable vcs-style "single compilation unit" semantics, where
    /// `` `define ``s declared in earlier groups are visible to later ones.
    pub fn set_single_unit(&mut self, enable: bool) {
        self.inner.pin_mut().set_single_unit(enable);
    }

    /// Toggle lenient (best-effort) parsing. When enabled, parse-time error
    /// diagnostics are not fatal: the indexer ingests whichever modules
    /// survived parsing. Mirrors pyslang's default policy and lets `bender
    /// kg build` survive repos with encrypted vendor IP or unsatisfied
    /// includes.
    pub fn set_lenient(&mut self, enable: bool) {
        self.inner.pin_mut().set_lenient(enable);
    }

    /// Parses one source group with scoped include directories and defines.
    pub fn parse_group(
        &mut self,
        files: &[String],
        includes: &[String],
        defines: &[String],
    ) -> Result<Vec<usize>> {
        let files_vec = files.to_vec();
        let includes_vec = normalize_include_dirs(includes)?;
        let defines_vec = defines.to_vec();

        let start = self.tree_count();
        self.inner
            .pin_mut()
            .parse_group(&files_vec, &includes_vec, &defines_vec)
            .map_err(|cause| SlangError::ParseGroup {
                message: cause.to_string(),
            })?;

        let end = self.tree_count();
        Ok((start..end).collect())
    }

    /// Returns the total number of parsed syntax trees in the session.
    pub fn tree_count(&self) -> usize {
        ffi::tree_count(self.inner.as_ref().unwrap())
    }

    /// Returns all parsed syntax trees in the session.
    pub fn all_trees(&self) -> Result<Vec<SyntaxTree<'_>>> {
        let count = self.tree_count();
        let mut out = Vec::with_capacity(count);
        for idx in 0..count {
            out.push(self.tree(idx)?);
        }
        Ok(out)
    }

    /// Returns the indices of syntax trees reachable from the given top modules.
    pub fn reachable_indices(&self, tops: &[String]) -> Result<Vec<usize>> {
        let tops = tops.to_vec();
        let indices =
            ffi::reachable_tree_indices(self.inner.as_ref().unwrap(), &tops).map_err(|cause| {
                SlangError::TrimByTop {
                    message: cause.to_string(),
                }
            })?;
        Ok(indices.into_iter().map(|i| i as usize).collect())
    }

    /// Returns syntax trees reachable from the given top modules.
    pub fn reachable_trees(&self, tops: &[String]) -> Result<Vec<SyntaxTree<'_>>> {
        let indices = self.reachable_indices(tops)?;
        let mut out = Vec::with_capacity(indices.len());
        for idx in indices {
            out.push(self.tree(idx)?);
        }
        Ok(out)
    }

    /// Prune the session's parsed trees to the given indices. After this
    /// call, every other API on the session (`tree_count`, `all_trees`,
    /// `walk_design`, `walk_elaborated`, ...) operates on the retained
    /// subset only. Out-of-range indices are silently skipped.
    pub fn retain_trees(&mut self, indices: &[u32]) {
        let owned: Vec<u32> = indices.to_vec();
        self.inner.pin_mut().retain_trees(&owned);
    }

    /// Returns a handle to the syntax tree at the given index.
    pub fn tree(&self, index: usize) -> Result<SyntaxTree<'_>> {
        Ok(SyntaxTree {
            inner: ffi::tree_at(self.inner.as_ref().unwrap(), index).map_err(|cause| {
                SlangError::TreeAccess {
                    message: cause.to_string(),
                }
            })?,
            _session: PhantomData,
        })
    }

    /// Walks every parsed syntax tree and emits structured records suitable
    /// for building a knowledge graph (module/package/interface declarations,
    /// instantiations, ports, parameters, and imports).
    pub fn walk_design(&self) -> Result<KgWalkResult> {
        ffi::walk_design(self.inner.as_ref().unwrap()).map_err(|cause| SlangError::Walk {
            message: cause.to_string(),
        })
    }

    /// Force elaboration from the given top modules and emit per-instance
    /// resolved parameter bindings and port widths.
    pub fn walk_elaborated(&self, tops: &[String]) -> Result<KgElabResult> {
        let tops_vec = tops.to_vec();
        ffi::walk_elaborated(self.inner.as_ref().unwrap(), &tops_vec).map_err(|cause| {
            SlangError::Walk {
                message: cause.to_string(),
            }
        })
    }
}

impl Default for SlangSession {
    fn default() -> Self {
        Self::new()
    }
}

impl SyntaxTreeRewriter {
    pub fn new() -> Self {
        Self {
            inner: ffi::new_syntax_tree_rewriter(),
        }
    }

    pub fn set_prefix(&mut self, prefix: impl Into<String>) {
        let prefix = prefix.into();
        self.inner.pin_mut().set_prefix(&prefix);
    }

    pub fn set_suffix(&mut self, suffix: impl Into<String>) {
        let suffix = suffix.into();
        self.inner.pin_mut().set_suffix(&suffix);
    }

    pub fn set_excludes(&mut self, excludes: Vec<String>) {
        self.inner.pin_mut().set_excludes(excludes);
    }

    pub fn rewrite_declarations<'a>(&mut self, tree: &SyntaxTree<'a>) -> SyntaxTree<'a> {
        SyntaxTree {
            inner: self
                .inner
                .pin_mut()
                .rewrite_declarations(tree.inner.clone()),
            _session: PhantomData,
        }
    }

    pub fn rewrite_references<'a>(&mut self, tree: &SyntaxTree<'a>) -> SyntaxTree<'a> {
        SyntaxTree {
            inner: self.inner.pin_mut().rewrite_references(tree.inner.clone()),
            _session: PhantomData,
        }
    }

    pub fn stats(&self) -> RenameStats {
        let rewriter = self.inner.as_ref().unwrap();
        RenameStats {
            renamed_declarations: ffi::renamed_declarations(rewriter),
            renamed_references: ffi::renamed_references(rewriter),
        }
    }
}

impl Default for SyntaxTreeRewriter {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(windows)]
fn normalize_include_dirs(includes: &[String]) -> Result<Vec<String>> {
    let mut out = Vec::with_capacity(includes.len());
    for include in includes {
        let canonical = dunce::canonicalize(include).map_err(|cause| SlangError::ParseGroup {
            message: format!(
                "Failed to canonicalize include directory '{}': {}",
                include, cause
            ),
        })?;
        out.push(canonical.to_string_lossy().into_owned());
    }
    Ok(out)
}

#[cfg(unix)]
fn normalize_include_dirs(includes: &[String]) -> Result<Vec<String>> {
    Ok(includes.to_vec())
}
