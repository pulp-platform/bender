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
#include <vector>

struct SlangPrintOpts;
struct SyntaxTrees;

class SlangContext {
  public:
    SlangContext();

    void set_includes(const rust::Vec<rust::String>& includes);
    void set_defines(const rust::Vec<rust::String>& defines);

    std::shared_ptr<slang::syntax::SyntaxTree> parse_file(rust::Str path);
    std::unique_ptr<SyntaxTrees> parse_files(const rust::Vec<rust::String>& paths);

  private:
    slang::SourceManager sourceManager;
    slang::parsing::PreprocessorOptions ppOptions;
    slang::DiagnosticEngine diagEngine;
    std::shared_ptr<slang::TextDiagnosticClient> diagClient;
};

std::unique_ptr<SlangContext> new_slang_context();

std::shared_ptr<slang::syntax::SyntaxTree> rename(std::shared_ptr<slang::syntax::SyntaxTree> tree, rust::Str prefix,
                                                  rust::Str suffix, const rust::Vec<rust::String>& excludes);

rust::String print_tree(std::shared_ptr<slang::syntax::SyntaxTree> tree, SlangPrintOpts options);

rust::String dump_tree_json(std::shared_ptr<slang::syntax::SyntaxTree> tree);

struct SyntaxTrees {
    std::vector<std::shared_ptr<slang::syntax::SyntaxTree>> trees;
};

std::unique_ptr<SyntaxTrees> new_syntax_trees();
void append_trees(SyntaxTrees& dst, const SyntaxTrees& src);
rust::Vec<std::uint32_t> reachable_tree_indices(const SyntaxTrees& trees, const rust::Vec<rust::String>& tops);
std::size_t tree_count(const SyntaxTrees& trees);
std::shared_ptr<slang::syntax::SyntaxTree> tree_at(const SyntaxTrees& trees, std::size_t index);

#endif // BENDER_SLANG_BRIDGE_H
