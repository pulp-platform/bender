// Copyright (c) 2025 ETH Zurich
// Tim Fischer <fischeti@iis.ee.ethz.ch>

#include "slang_bridge.h"

#include "bender-slang/src/lib.rs.h"
#include "slang/syntax/SyntaxPrinter.h"
#include "slang/syntax/SyntaxVisitor.h"

using namespace slang;
using namespace slang::driver;
using namespace slang::syntax;

using std::memcpy;
using std::shared_ptr;
using std::string;
using std::string_view;
using std::vector;

// Create a new SlangContext instance
std::unique_ptr<SlangContext> new_slang_context() { return std::make_unique<SlangContext>(); }

// Constructor: initialize driver with standard args
SlangContext::SlangContext() { driver.addStandardArgs(); }

// Add a source file path to the context
void SlangContext::add_source(rust::Str path) { sources.emplace_back(std::string(path)); }

// Add an include path to the context
void SlangContext::add_include(rust::Str path) { includes.emplace_back(std::string(path)); }

// Add a define to the context
void SlangContext::add_define(rust::Str def) { defines.emplace_back(std::string(def)); }

bool SlangContext::parse() {
    vector<string> arg_strings;
    arg_strings.push_back("slang_tool");

    for (const auto& s : sources)
        arg_strings.push_back(s);
    for (const auto& i : includes) {
        arg_strings.push_back("-I");
        arg_strings.push_back(i);
    }
    for (const auto& d : defines) {
        arg_strings.push_back("-D");
        arg_strings.push_back(d);
    }

    vector<const char*> c_args;
    for (const auto& s : arg_strings)
        c_args.push_back(s.c_str());

    if (!driver.parseCommandLine(c_args.size(), c_args.data())) {
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

// Get the number of syntax trees parsed by the driver
size_t SlangContext::get_tree_count() const { return driver.syntaxTrees.size(); }

// Get the syntax tree at the specified index
shared_ptr<SyntaxTree> SlangContext::get_tree(size_t index) const { return driver.syntaxTrees[index]; }

// Rewriter that adds prefix/suffix to module and instantiated hierarchy names
class SuffixPrefixRewriter : public SyntaxRewriter<SuffixPrefixRewriter> {
  public:
    SuffixPrefixRewriter(string_view prefix, string_view suffix) : prefix(prefix), suffix(suffix) {}

    // Helper to allocate and build renamed string with prefix/suffix
    string_view rename(string_view name) {
        size_t len = prefix.size() + name.size() + suffix.size();
        char* mem = (char*)alloc.allocate(len, 1);
        memcpy(mem, prefix.data(), prefix.size());
        memcpy(mem + prefix.size(), name.data(), name.size());
        memcpy(mem + prefix.size() + name.size(), suffix.data(), suffix.size());
        return string_view(mem, len);
    }

    // Renames "module foo;" -> "module <prefix>foo<suffix>;"
    void handle(const ModuleDeclarationSyntax& node) {
        if (node.header->name.isMissing())
            return;

        // Create a new name token
        auto newName = rename(node.header->name.valueText());
        auto newNameToken = makeId(newName, node.header->name.trivia());

        // Clone the header and update the name
        ModuleHeaderSyntax* newHeader = deepClone(*node.header, alloc);
        newHeader->name = newNameToken;

        // Replace the old header with the new one
        replace(*node.header, *newHeader);

        // Continue visiting child nodes
        visitDefault(node);
    }

    // Renames "foo i_foo();" -> "<prefix>foo<suffix> i_foo();"
    void handle(const HierarchyInstantiationSyntax& node) {
        // Check to make sure we are dealing with an identifier
        // and not a built-in type e.g. `initial foo();`
        if (node.type.kind == parsing::TokenKind::Identifier) {

            // Create a new name token
            auto newName = rename(node.type.valueText());
            auto newNameToken = makeId(newName);

            // Clone the node and update the type token
            HierarchyInstantiationSyntax* newNode = deepClone(node, alloc);
            newNode->type = newNameToken;

            // Replace the old node with the new one
            replace(node, *newNode, true);
        }

        // Continue visiting child nodes
        visitDefault(node);
    }

  private:
    string_view prefix;
    string_view suffix;
};

// Rename modules and instantiated hierarchy names in the given syntax tree
shared_ptr<SyntaxTree> SlangContext::rename_tree(const shared_ptr<SyntaxTree> tree, rust::Str prefix,
                                                 rust::Str suffix) const {

    // Convert rust::Str to string_view and instantiate rewriter
    string_view prefix_str(prefix.data(), prefix.size());
    string_view suffix_str(suffix.data(), suffix.size());
    SuffixPrefixRewriter rewriter(prefix_str, suffix_str);

    // Apply the rewriter to the tree and return the transformed tree
    return rewriter.transform(tree);
}

// Print the given syntax tree with specified options
rust::String SlangContext::print_tree(const shared_ptr<SyntaxTree> tree, SlangPrintOpts options) const {

    // Set up the printer with options
    SyntaxPrinter printer(driver.sourceManager);

    printer.setIncludeDirectives(options.include_directives);
    printer.setExpandIncludes(options.expand_includes);
    printer.setExpandMacros(options.expand_macros);
    printer.setSquashNewlines(options.squash_newlines);
    printer.setIncludeComments(options.include_comments);

    // Print the tree root and return as rust::String
    printer.print(tree->root());
    return rust::String(printer.str());
}
