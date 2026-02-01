// Copyright (c) 2025 ETH Zurich
// Tim Fischer <fischeti@iis.ee.ethz.ch>

#include "slang_bridge.h"
#include "bender-slang/src/lib.rs.h"
#include "slang/syntax/SyntaxPrinter.h"
#include <memory>

using namespace slang;
using namespace slang::driver;
using namespace slang::syntax;

SlangContext::SlangContext() {
    driver.addStandardArgs();
}

void SlangContext::add_source(rust::Str path) {
    sources.emplace_back(std::string(path));
}

void SlangContext::add_include(rust::Str path) {
    includes.emplace_back(std::string(path));
}

void SlangContext::add_define(rust::Str def) {
    defines.emplace_back(std::string(def));
}

bool SlangContext::parse() {
    // Construct argv for the driver
    std::vector<std::string> arg_strings;
    arg_strings.push_back("slang_tool");

    for (const auto& s : sources) arg_strings.push_back(s);
    for (const auto& i : includes) { arg_strings.push_back("-I"); arg_strings.push_back(i); }
    for (const auto& d : defines) { arg_strings.push_back("-D"); arg_strings.push_back(d); }

    std::vector<const char*> c_args;
    for (const auto& s : arg_strings) c_args.push_back(s.c_str());

    if (!driver.parseCommandLine(c_args.size(), c_args.data())) {
        // You might want to capture stderr here or throw a clearer error
        throw std::runtime_error("Failed to parse command line args");
    }

    if (!driver.processOptions()) {
        throw std::runtime_error("Failed to process options");
    }

    bool ok = driver.parseAllSources();
    // reportDiagnostics returns true if issues found, so we invert logic or check strictness
    bool hasErrors = driver.reportDiagnostics(false);

    return ok && !hasErrors;
}

size_t SlangContext::get_tree_count() const {
    return driver.syntaxTrees.size();
}

std::shared_ptr<slang::syntax::SyntaxTree> SlangContext::get_tree(size_t index) const {
    if (index >= driver.syntaxTrees.size()) {
        // Rust's loop bounds prevent this, but good for safety
        throw std::out_of_range("Syntax tree index out of range");
    }
    return driver.syntaxTrees[index];
}

rust::String SlangContext::print_tree(const SyntaxTree& tree, SlangPrintOpts options) const {
    // Use the SourceManager from the driver (this context)
    SyntaxPrinter printer(driver.sourceManager);

    printer.setIncludeDirectives(options.include_directives);
    printer.setExpandIncludes(options.expand_includes);
    printer.setExpandMacros(options.expand_macros);
    printer.setSquashNewlines(options.squash_newlines);
    printer.setIncludeComments(options.include_comments);

    printer.print(tree);
    return rust::String(printer.str());
}

std::unique_ptr<SlangContext> new_slang_context() {
    return std::make_unique<SlangContext>();
}
