// Copyright (c) 2025 ETH Zurich
// Tim Fischer <fischeti@iis.ee.ethz.ch>

#include "slang_bridge.h"

#include "bender-slang/src/lib.rs.h"
#include "slang/diagnostics/DiagnosticEngine.h"
#include "slang/diagnostics/TextDiagnosticClient.h"
#include "slang/syntax/CSTSerializer.h"
#include "slang/syntax/SyntaxPrinter.h"
#include "slang/syntax/SyntaxVisitor.h"
#include "slang/text/Json.h"

#include <functional>
#include <stdexcept>
#include <unordered_map>
#include <unordered_set>

using namespace slang;
using namespace slang::driver;
using namespace slang::syntax;
using namespace slang::parsing;

using std::memcpy;
using std::shared_ptr;
using std::string;
using std::string_view;
using std::vector;

std::unique_ptr<SlangSession> new_slang_session() { return std::make_unique<SlangSession>(); }

SlangContext::SlangContext() : diagEngine(sourceManager), diagClient(std::make_shared<TextDiagnosticClient>()) {
    diagEngine.addClient(diagClient);
}

// Set the include paths for the preprocessor
void SlangContext::set_includes(const rust::Vec<rust::String>& incs) {
    for (const auto& inc : incs) {
        std::string incStr(inc.data(), inc.size());
        if (auto ec = sourceManager.addUserDirectories(incStr); ec) {
            throw std::runtime_error("Failed to add include directory '" + incStr + "': " + ec.message());
        }
    }
}

// Sets the preprocessor defines
void SlangContext::set_defines(const rust::Vec<rust::String>& defs) {
    ppOptions.predefines.reserve(defs.size());
    for (const auto& def : defs) {
        ppOptions.predefines.emplace_back(def.data(), def.size());
    }
}

std::vector<std::shared_ptr<SyntaxTree>> SlangContext::parse_files(const rust::Vec<rust::String>& paths) {
    Bag options;
    options.set(ppOptions);

    std::vector<std::shared_ptr<SyntaxTree>> out;
    out.reserve(paths.size());

    for (const auto& path : paths) {
        string_view pathView(path.data(), path.size());
        auto result = SyntaxTree::fromFile(pathView, sourceManager, options);

        if (!result) {
            auto& err = result.error();
            std::string msg = "System Error loading '" + std::string(err.second) + "': " + err.first.message();
            throw std::runtime_error(msg);
        }

        auto tree = *result;
        diagClient->clear();
        diagEngine.clearIncludeStack();

        bool hasErrors = false;
        for (const auto& diag : tree->diagnostics()) {
            hasErrors |= diag.isError();
            diagEngine.issue(diag);
        }

        if (hasErrors) {
            std::string rendered = diagClient->getString();
            if (rendered.empty()) {
                rendered = "Failed to parse '" + std::string(pathView) + "'.";
            }
            throw std::runtime_error(rendered);
        }

        out.push_back(tree);
    }

    return out;
}

void SlangSession::parse_group(const rust::Vec<rust::String>& files, const rust::Vec<rust::String>& includes,
                               const rust::Vec<rust::String>& defines) {
    auto ctx = std::make_unique<SlangContext>();
    ctx->set_includes(includes);
    ctx->set_defines(defines);

    auto parsed = ctx->parse_files(files);
    allTrees.reserve(allTrees.size() + parsed.size());
    for (const auto& tree : parsed) {
        allTrees.push_back(tree);
    }

    contexts.push_back(std::move(ctx));
}

// Rewriter that adds prefix/suffix to module and instantiated hierarchy names
class SuffixPrefixRewriter : public SyntaxRewriter<SuffixPrefixRewriter> {
  public:
    SuffixPrefixRewriter(string_view prefix, string_view suffix, const std::unordered_set<std::string>& excludes)
        : prefix(prefix), suffix(suffix), excludes(excludes) {}

    // Helper to allocate and build renamed string with prefix/suffix
    string_view rename(string_view name) {
        if (excludes.count(std::string(name))) {
            return name;
        }
        size_t len = prefix.size() + name.size() + suffix.size();
        char* mem = (char*)alloc.allocate(len, 1);
        memcpy(mem, prefix.data(), prefix.size());
        memcpy(mem + prefix.size(), name.data(), name.size());
        memcpy(mem + prefix.size() + name.size(), suffix.data(), suffix.size());
        return string_view(mem, len);
    }

    // Renames "module foo;" -> "module <prefix>foo<suffix>;"
    // Note: Handles packages and interfaces too.
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
    // Note: Handles modules and interfaces.
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

    // Renames "import foo;" -> "import <prefix>foo<suffix>;"
    void handle(const PackageImportItemSyntax& node) {
        if (node.package.isMissing())
            return;

        auto newName = rename(node.package.valueText());
        auto newNameToken = makeId(newName, node.package.trivia());

        PackageImportItemSyntax* newNode = deepClone(node, alloc);
        newNode->package = newNameToken;

        replace(node, *newNode);
        visitDefault(node);
    }

    // Renames "virtual MyIntf foo;" -> "virtual <prefix>MyIntf<suffix> foo;"
    void handle(const VirtualInterfaceTypeSyntax& node) {
        if (node.name.isMissing())
            return;

        auto newName = rename(node.name.valueText());
        auto newNameToken = makeId(newName, node.name.trivia());

        VirtualInterfaceTypeSyntax* newNode = deepClone(node, alloc);
        newNode->name = newNameToken;

        replace(node, *newNode);
        visitDefault(node);
    }

    // Renames "foo::bar" -> "<prefix>foo<suffix>::bar"
    void handle(const ScopedNameSyntax& node) {
        // Only rename if the left side is a simple identifier (e.g., a package name)
        // We ignore nested calls or parameterized classes for now.
        if (node.left->kind == SyntaxKind::IdentifierName) {
            auto& leftNode = node.left->as<IdentifierNameSyntax>();
            auto name = leftNode.identifier.valueText();

            // Skip built-in keywords that look like scopes
            if (name != "$unit" && name != "local" && name != "super" && name != "this") {
                auto newName = rename(name);
                auto newNameToken = makeId(newName, leftNode.identifier.trivia());

                // Clone the left node and update identifier
                IdentifierNameSyntax* newLeft = deepClone(leftNode, alloc);
                newLeft->identifier = newNameToken;

                // Clone the scoped node and attach new left
                ScopedNameSyntax* newNode = deepClone(node, alloc);
                newNode->left = newLeft;

                replace(node, *newNode);
            }
        }

        // Visit children to handle recursive scopes
        // e.g., OuterPkg::InnerPkg::Item
        visitDefault(node);
    }

  private:
    string_view prefix;
    string_view suffix;
    const std::unordered_set<std::string>& excludes;
};

// Transform the given syntax tree by renaming modules and instantiated hierarchy names with the specified prefix/suffix
std::shared_ptr<SyntaxTree> rename(std::shared_ptr<SyntaxTree> tree, rust::Str prefix, rust::Str suffix,
                                   const rust::Vec<rust::String>& excludes) {
    std::string_view p(prefix.data(), prefix.size());
    std::string_view s(suffix.data(), suffix.size());

    std::unordered_set<std::string> excludeSet;
    for (const auto& e : excludes) {
        excludeSet.insert(std::string(e));
    }

    // SuffixPrefixRewriter is defined in the .cpp file as before
    SuffixPrefixRewriter rewriter(p, s, excludeSet);
    return rewriter.transform(tree);
}

// Print the given syntax tree with specified options
rust::String print_tree(const shared_ptr<SyntaxTree> tree, SlangPrintOpts options) {

    // Set up the printer with options
    SyntaxPrinter printer(tree->sourceManager());

    printer.setIncludeDirectives(true);
    printer.setExpandIncludes(true);
    printer.setExpandMacros(options.expand_macros);
    printer.setSquashNewlines(options.squash_newlines);
    printer.setIncludeComments(options.include_comments);

    // Print the tree root and return as rust::String
    printer.print(tree->root());
    return rust::String(printer.str());
}

// Dumps the AST/CST to a JSON string
rust::String dump_tree_json(std::shared_ptr<SyntaxTree> tree) {
    JsonWriter writer;
    writer.setPrettyPrint(true);

    // CSTSerializer is the class Slang uses to convert AST -> JSON
    CSTSerializer serializer(writer);

    // Serialize the specific tree root
    serializer.serialize(*tree);

    // Convert string_view to rust::String
    return rust::String(std::string(writer.view()));
}

rust::Vec<std::uint32_t> reachable_tree_indices(const SlangSession& session, const rust::Vec<rust::String>& tops) {
    const auto& treeVec = session.trees();

    // Build a mapping from declared symbol names to the index of the tree that declares them
    std::unordered_map<std::string_view, size_t> nameToTreeIndex;
    for (size_t i = 0; i < treeVec.size(); ++i) {
        const auto& metadata = treeVec[i]->getMetadata();
        for (auto name : metadata.getDeclaredSymbols()) {
            nameToTreeIndex.emplace(name, i);
        }
    }

    // Build a dependency graph where each tree points to the trees that declare symbols it references
    std::vector<std::vector<size_t>> deps(treeVec.size());
    for (size_t i = 0; i < treeVec.size(); ++i) {
        const auto& metadata = treeVec[i]->getMetadata();
        std::unordered_set<size_t> seen;
        for (auto ref : metadata.getReferencedSymbols()) {
            auto it = nameToTreeIndex.find(ref);
            // Avoid duplicate dependencies in case of multiple references to the same symbol
            if (it != nameToTreeIndex.end() && seen.insert(it->second).second) {
                deps[i].push_back(it->second);
            }
        }
    }

    // Map the top module names to their corresponding tree indices
    std::vector<size_t> startIndices;
    startIndices.reserve(tops.size());
    for (const auto& top : tops) {
        std::string_view name(top.data(), top.size());
        auto it = nameToTreeIndex.find(name);
        if (it == nameToTreeIndex.end()) {
            throw std::runtime_error("Top module not found in any parsed source file: " + std::string(name));
        } else {
            startIndices.push_back(it->second);
        }
    }

    // Perform a DFS from the top modules to find all reachable trees
    std::vector<bool> reachable(treeVec.size(), false);
    std::function<void(size_t)> dfs = [&](size_t index) {
        if (reachable[index]) {
            return;
        }
        reachable[index] = true;
        for (auto dep : deps[index]) {
            dfs(dep);
        }
    };

    for (auto start : startIndices) {
        dfs(start);
    }

    // Collect the indices of reachable trees and return as rust::Vec
    rust::Vec<std::uint32_t> result;
    for (size_t i = 0; i < reachable.size(); ++i) {
        if (reachable[i]) {
            result.push_back(static_cast<std::uint32_t>(i));
        }
    }
    return result;
}

std::size_t tree_count(const SlangSession& session) { return session.trees().size(); }

std::shared_ptr<SyntaxTree> tree_at(const SlangSession& session, std::size_t index) {
    if (index >= session.trees().size()) {
        throw std::runtime_error("Tree index out of bounds.");
    }
    return session.trees()[index];
}
