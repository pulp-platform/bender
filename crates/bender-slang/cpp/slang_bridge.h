// Copyright (c) 2025 ETH Zurich
// Tim Fischer <fischeti@iis.ee.ethz.ch>

#pragma once
#include "rust/cxx.h"
#include "slang/driver/Driver.h"
#include "slang/syntax/SyntaxTree.h"

#include <memory>
#include <string>
#include <vector>

struct SlangPrintOpts; // Forward decl

// The wrapper class exposed as "SlangContext" to Rust
class SlangContext {
  public:
    SlangContext();

    void add_source(rust::Str path);
    void add_include(rust::Str path);
    void add_define(rust::Str def);

    bool parse();

    size_t get_tree_count() const;
    std::shared_ptr<slang::syntax::SyntaxTree> get_tree(size_t index) const;

    std::shared_ptr<slang::syntax::SyntaxTree> rename_tree(const std::shared_ptr<slang::syntax::SyntaxTree>,
                                                           rust::Str prefix, rust::Str suffix) const;

    rust::String print_tree(const std::shared_ptr<slang::syntax::SyntaxTree>, SlangPrintOpts options) const;

  private:
    slang::driver::Driver driver;

    // We buffer args to pass to driver.parseCommandLine later
    std::vector<std::string> sources;
    std::vector<std::string> includes;
    std::vector<std::string> defines;
};

std::unique_ptr<SlangContext> new_slang_context();
