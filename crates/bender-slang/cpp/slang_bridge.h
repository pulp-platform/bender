// Copyright (c) 2025 ETH Zurich
// Tim Fischer <fischeti@iis.ee.ethz.ch>

#ifndef BENDER_SLANG_BRIDGE_H
#define BENDER_SLANG_BRIDGE_H

#include "rust/cxx.h"
#include "slang/diagnostics/DiagnosticEngine.h"
#include "slang/diagnostics/TextDiagnosticClient.h"
#include "slang/parsing/Preprocessor.h"
#include "slang/syntax/SyntaxTree.h"
#include "slang/text/SourceManager.h"

#include <cstddef>
#include <cstdint>
#include <memory>
#include <string>
#include <unordered_map>
#include <unordered_set>
#include <vector>

// The kg walker shared types (KgPort/KgParam/KgInstance/KgImport/KgModule/
// KgKeyValue/KgWalkResult) are emitted by cxx with their full definitions in
// bender-slang/src/lib.rs.h. We forward declare them here so the prototype of
// `walk_design` is visible to the cxx-generated bridge .cc, where the cxx
// translation unit also contains the full type definitions afterwards.
struct KgKeyValue;
struct KgParam;
struct KgPort;
struct KgInstance;
struct KgImport;
struct KgModule;
struct KgWalkResult;
struct KgInstanceContext;
struct KgElabResult;

struct SlangPrintOpts;

// A SlangContext wraps a per-group preprocessor configuration. The
// SourceManager is owned by the parent SlangSession and shared across all
// groups so the resulting SyntaxTrees can be combined into a single
// `slang::ast::Compilation` for elaboration.
class SlangContext {
  public:
    explicit SlangContext(slang::SourceManager& sm);

    void set_includes(const rust::Vec<rust::String>& includes);
    void set_defines(const rust::Vec<rust::String>& defines);

    // Parse `paths` into a single SyntaxTree (one slang compilation unit).
    // When `inherited` is non-empty, slang predefines those macros into the
    // preprocessor before parsing, propagating `define`s declared in earlier
    // source groups (used to emulate vcs/`vlog -mfcu` "single-unit" mode).
    // When `lenient` is true, parse-time error diagnostics are reported on
    // the diagnostic engine but do NOT abort: the partially built syntax
    // tree is returned and downstream walks ingest whatever survived. This
    // matches pyslang's best-effort policy for hostile inputs (encrypted
    // vendor IP, missing includes, ...).
    std::vector<std::shared_ptr<slang::syntax::SyntaxTree>>
    parse_files(const rust::Vec<rust::String>& paths,
                slang::syntax::SyntaxTree::MacroList inherited,
                bool lenient);

  private:
    slang::SourceManager& sourceManager;
    slang::parsing::PreprocessorOptions ppOptions;
    slang::DiagnosticEngine diagEngine;
    std::shared_ptr<slang::TextDiagnosticClient> diagClient;
};

class SlangSession {
  public:
    void parse_group(const rust::Vec<rust::String>& files, const rust::Vec<rust::String>& includes,
                     const rust::Vec<rust::String>& defines);

    // Toggle cross-group macro propagation. When true, every subsequent
    // `parse_group` call inherits all `define`s collected from prior groups,
    // matching `vcs` (default) / `vlog -mfcu` semantics.
    void set_single_unit(bool enable) { singleUnit = enable; }
    bool single_unit() const { return singleUnit; }

    // Toggle lenient (best-effort) parsing. When true, parse-time error
    // diagnostics are still reported but do NOT abort the build; whatever
    // syntax was successfully recovered is retained for downstream walks.
    void set_lenient(bool enable) { lenient = enable; }
    bool is_lenient() const { return lenient; }

    const std::vector<std::shared_ptr<slang::syntax::SyntaxTree>>& trees() const { return allTrees; }
    slang::SourceManager& source_manager() { return sourceManager; }

    // Keep only the syntax trees at the given indices (in-place); drop the
    // rest. Used by the kg pipeline to prune to the set of trees reachable
    // from one or more top modules before the downstream walks. Subsequent
    // `walk_design` / `walk_elaborated` calls operate on the pruned set.
    void retain_trees(const rust::Vec<std::uint32_t>& indices);

  private:
    slang::SourceManager sourceManager;
    std::vector<std::unique_ptr<SlangContext>> contexts;
    std::vector<std::shared_ptr<slang::syntax::SyntaxTree>> allTrees;
    // Macros defined by previously-parsed groups; backed by SyntaxTree
    // ownership in `allTrees` so the pointers remain valid.
    std::vector<const slang::syntax::DefineDirectiveSyntax*> accumulatedMacros;
    bool singleUnit = false;
    bool lenient = false;
};

class SyntaxTreeRewriter {
  public:
    void set_prefix(rust::Str prefix);
    void set_suffix(rust::Str suffix);
    void set_excludes(const rust::Vec<rust::String> excludes);

    std::shared_ptr<slang::syntax::SyntaxTree> rewrite_declarations(std::shared_ptr<slang::syntax::SyntaxTree> tree);
    std::shared_ptr<slang::syntax::SyntaxTree> rewrite_references(std::shared_ptr<slang::syntax::SyntaxTree> tree);

    std::uint64_t renamed_declarations() const { return renamedDeclarations; }
    std::uint64_t renamed_references() const { return renamedReferences; }

  private:
    std::string prefix;
    std::string suffix;
    std::unordered_set<std::string> excludes;
    std::unordered_map<std::string, std::string> renameMap;
    std::uint64_t renamedDeclarations = 0;
    std::uint64_t renamedReferences = 0;
};

std::unique_ptr<SlangSession> new_slang_session();
std::unique_ptr<SyntaxTreeRewriter> new_syntax_tree_rewriter();

rust::String print_tree(std::shared_ptr<slang::syntax::SyntaxTree> tree, SlangPrintOpts options);

rust::String dump_tree_json(std::shared_ptr<slang::syntax::SyntaxTree> tree);

rust::Vec<std::uint32_t> reachable_tree_indices(const SlangSession& session, const rust::Vec<rust::String>& tops);
std::size_t tree_count(const SlangSession& session);
std::shared_ptr<slang::syntax::SyntaxTree> tree_at(const SlangSession& session, std::size_t index);
std::uint64_t renamed_declarations(const SyntaxTreeRewriter& rewriter);
std::uint64_t renamed_references(const SyntaxTreeRewriter& rewriter);

// Walk every parsed syntax tree in `session` and emit knowledge-graph records.
// Defined in cpp/walker.cpp; the cxx bridge generates the Rust-side glue and
// the matching definition of KgWalkResult in bender-slang/src/lib.rs.h.
KgWalkResult walk_design(const SlangSession& session);

// Build a slang::ast::Compilation from the session's parsed trees, force
// elaboration from the given tops, and harvest resolved parameter bindings
// and port widths for each elaborated InstanceSymbol. Defined in cpp/elab.cpp.
KgElabResult walk_elaborated(const SlangSession& session, const rust::Vec<rust::String>& tops);

#endif // BENDER_SLANG_BRIDGE_H
