// Copyright (c) 2025 ETH Zurich
// Tim Fischer <fischeti@iis.ee.ethz.ch>

#ifndef BENDER_SLANG_BRIDGE_H
#define BENDER_SLANG_BRIDGE_H

#include "rust/cxx.h"
#include "slang/driver/Driver.h"
#include "slang/syntax/SyntaxTree.h"

#include <memory>
#include <string>
#include <vector>

struct SlangPrintOpts;

class SlangContext {
  public:
    SlangContext();

    void set_includes(const rust::Vec<rust::String>& includes);
    void set_defines(const rust::Vec<rust::String>& defines);

    std::shared_ptr<slang::syntax::SyntaxTree> parse_file(rust::Str path);

  private:
    slang::SourceManager sourceManager;
    slang::parsing::PreprocessorOptions ppOptions;
};

std::unique_ptr<SlangContext> new_slang_context();

std::shared_ptr<slang::syntax::SyntaxTree> rename(std::shared_ptr<slang::syntax::SyntaxTree> tree, rust::Str prefix,
                                                  rust::Str suffix, const rust::Vec<rust::String>& excludes);

rust::String print_tree(std::shared_ptr<slang::syntax::SyntaxTree> tree, SlangPrintOpts options);

rust::String dump_tree_json(std::shared_ptr<slang::syntax::SyntaxTree> tree);

#endif // BENDER_SLANG_BRIDGE_H
