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

std::unique_ptr<SlangSession> new_slang_session();

std::shared_ptr<slang::syntax::SyntaxTree> rename(std::shared_ptr<slang::syntax::SyntaxTree> tree, rust::Str prefix,
                                                  rust::Str suffix, const rust::Vec<rust::String>& excludes);

rust::String print_tree(std::shared_ptr<slang::syntax::SyntaxTree> tree, SlangPrintOpts options);

rust::String dump_tree_json(std::shared_ptr<slang::syntax::SyntaxTree> tree);

rust::Vec<std::uint32_t> reachable_tree_indices(const SlangSession& session, const rust::Vec<rust::String>& tops);
std::size_t tree_count(const SlangSession& session);
std::shared_ptr<slang::syntax::SyntaxTree> tree_at(const SlangSession& session, std::size_t index);

#endif // BENDER_SLANG_BRIDGE_H
