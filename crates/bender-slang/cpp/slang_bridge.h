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

struct SlangPrintOpts;
struct ParsedTree;

// Internal per-tree record kept by the session. Plain C++ (no cxx types) so the session can own
// it without depending on the generated header; the bridge functions convert it to `ParsedTree`
// at the FFI boundary.
struct TreeEntry {
    std::shared_ptr<slang::syntax::SyntaxTree> tree;
    // The source path exactly as it was handed to parse_group (so the Rust side can match it
    // back to its own SourceFile entry without a separate index map).
    std::string path;
    // False if slang reported any error diagnostic while parsing this file.
    bool parsedOk = true;
    // True if slang emitted at least one `pragma protect` envelope diagnostic (the
    // lexer/preprocessor signal that the file contains IEEE-1735 encrypted content). Lets the
    // Rust side discriminate "encrypted IP" (auto-tolerated) from "real syntax bug" (fail by
    // default; tolerate with --allow-broken).
    bool encrypted = false;
};

class SlangContext {
  public:
    SlangContext();

    void set_includes(const rust::Vec<rust::String>& includes);
    void set_defines(const rust::Vec<rust::String>& defines);

    std::vector<TreeEntry> parse_files(const rust::Vec<rust::String>& paths);

  private:
    slang::SourceManager sourceManager;
    slang::parsing::PreprocessorOptions ppOptions;
    slang::DiagnosticEngine diagEngine;
    std::shared_ptr<slang::TextDiagnosticClient> diagClient;
};

class SlangSession {
  public:
    void parse_group(const rust::Vec<rust::String>& files, const rust::Vec<rust::String>& includes,
                     const rust::Vec<rust::String>& defines);

    const std::vector<TreeEntry>& entries() const { return treeEntries; }

  private:
    std::vector<std::unique_ptr<SlangContext>> contexts;
    std::vector<TreeEntry> treeEntries;
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

std::size_t tree_count(const SlangSession& session);
rust::Vec<ParsedTree> all_trees(const SlangSession& session);
rust::Vec<ParsedTree> reachable_trees(const SlangSession& session, const rust::Vec<rust::String>& tops);
rust::Vec<rust::String> resolved_include_paths_for(const rust::Vec<ParsedTree>& trees);
std::uint64_t renamed_declarations(const SyntaxTreeRewriter& rewriter);
std::uint64_t renamed_references(const SyntaxTreeRewriter& rewriter);

#endif // BENDER_SLANG_BRIDGE_H
