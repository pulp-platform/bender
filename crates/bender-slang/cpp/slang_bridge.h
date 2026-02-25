// Copyright (c) 2025 ETH Zurich
// Tim Fischer <fischeti@iis.ee.ethz.ch>

#ifndef BENDER_SLANG_BRIDGE_H
#define BENDER_SLANG_BRIDGE_H

#include "rust/cxx.h"
#include "slang/diagnostics/DiagnosticEngine.h"
#include "slang/diagnostics/TextDiagnosticClient.h"
#include "slang/driver/Driver.h"
#include "slang/syntax/SyntaxTree.h"

#include <cstddef>
#include <cstdint>
#include <memory>
#include <string>
#include <unordered_map>
#include <unordered_set>
#include <vector>

struct SlangPrintOpts;

class SlangContext {
  public:
    SlangContext();

    void set_includes(const rust::Vec<rust::String>& includes);
    void set_defines(const rust::Vec<rust::String>& defines);

    std::vector<std::shared_ptr<slang::syntax::SyntaxTree>> parse_files(const rust::Vec<rust::String>& paths);

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

    const std::vector<std::shared_ptr<slang::syntax::SyntaxTree>>& trees() const { return allTrees; }

  private:
    std::vector<std::unique_ptr<SlangContext>> contexts;
    std::vector<std::shared_ptr<slang::syntax::SyntaxTree>> allTrees;
};

class SyntaxTreeRewriter {
  public:
    void reset_rename_map();
    void register_declarations(std::shared_ptr<slang::syntax::SyntaxTree> tree);
    void set_prefix(rust::Str prefix);
    void set_suffix(rust::Str suffix);
    void set_excludes(const rust::Vec<rust::String> excludes);

    std::shared_ptr<slang::syntax::SyntaxTree> rewrite_tree(std::shared_ptr<slang::syntax::SyntaxTree> tree);

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

#endif // BENDER_SLANG_BRIDGE_H
