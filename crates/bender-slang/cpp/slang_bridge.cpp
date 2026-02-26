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

using std::shared_ptr;
using std::string;
using std::string_view;
using std::vector;

std::unique_ptr<SlangSession> new_slang_session() { return std::make_unique<SlangSession>(); }
std::unique_ptr<SyntaxTreeRewriter> new_syntax_tree_rewriter() { return std::make_unique<SyntaxTreeRewriter>(); }

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

class DeclarationCollector : public SyntaxVisitor<DeclarationCollector> {
  public:
    explicit DeclarationCollector(std::unordered_set<std::string>& names) : names(names) {}

    void handle(const ModuleDeclarationSyntax& node) {
        if (!node.header->name.isMissing()) {
            names.insert(std::string(node.header->name.valueText()));
        }
        visitDefault(node);
    }

  private:
    std::unordered_set<std::string>& names;
};

// Rewriter that renames declarations and references only if their declaration
// exists in the precomputed renameMap.
class MappedRewriter : public SyntaxRewriter<MappedRewriter> {
  public:
    MappedRewriter(const std::unordered_map<std::string, std::string>& renameMap, std::uint64_t& declRenamed,
                   std::uint64_t& refRenamed)
        : renameMap(renameMap), declRenamed(declRenamed), refRenamed(refRenamed) {}

    string_view mapped_name(string_view name) {
        auto it = renameMap.find(std::string(name));
        if (it == renameMap.end()) {
            return {};
        }
        return string_view(it->second);
    }

    void handle(const ModuleDeclarationSyntax& node) {
        if (node.header->name.isMissing())
            return;

        auto newName = mapped_name(node.header->name.valueText());
        if (newName.empty()) {
            visitDefault(node);
            return;
        }

        auto newNameToken = node.header->name.withRawText(alloc, newName);

        ModuleHeaderSyntax* newHeader = deepClone(*node.header, alloc);
        newHeader->name = newNameToken;

        replace(*node.header, *newHeader);
        declRenamed++;
        visitDefault(node);
    }

    void handle(const HierarchyInstantiationSyntax& node) {
        if (node.type.kind == parsing::TokenKind::Identifier) {
            auto newName = mapped_name(node.type.valueText());
            if (!newName.empty()) {
                auto newNameToken = node.type.withRawText(alloc, newName);

                HierarchyInstantiationSyntax* newNode = deepClone(node, alloc);
                newNode->type = newNameToken;

                replace(node, *newNode, true);
                refRenamed++;
            }
        }

        visitDefault(node);
    }

    void handle(const PackageImportItemSyntax& node) {
        if (node.package.isMissing())
            return;

        auto newName = mapped_name(node.package.valueText());
        if (newName.empty()) {
            visitDefault(node);
            return;
        }
        auto newNameToken = node.package.withRawText(alloc, newName);

        PackageImportItemSyntax* newNode = deepClone(node, alloc);
        newNode->package = newNameToken;

        replace(node, *newNode);
        refRenamed++;
        visitDefault(node);
    }

    void handle(const VirtualInterfaceTypeSyntax& node) {
        if (node.name.isMissing())
            return;

        auto newName = mapped_name(node.name.valueText());
        if (newName.empty()) {
            visitDefault(node);
            return;
        }
        auto newNameToken = node.name.withRawText(alloc, newName);

        VirtualInterfaceTypeSyntax* newNode = deepClone(node, alloc);
        newNode->name = newNameToken;

        replace(node, *newNode);
        refRenamed++;
        visitDefault(node);
    }

    void handle(const ScopedNameSyntax& node) {
        if (node.left->kind == SyntaxKind::IdentifierName) {
            auto& leftNode = node.left->as<IdentifierNameSyntax>();
            auto name = leftNode.identifier.valueText();

            if (name != "$unit" && name != "local" && name != "super" && name != "this") {
                auto newName = mapped_name(name);
                if (!newName.empty()) {
                    auto newNameToken = leftNode.identifier.withRawText(alloc, newName);

                    IdentifierNameSyntax* newLeft = deepClone(leftNode, alloc);
                    newLeft->identifier = newNameToken;

                    ScopedNameSyntax* newNode = deepClone(node, alloc);
                    newNode->left = newLeft;

                    replace(node, *newNode);
                    refRenamed++;
                }
            }
        }

        visitDefault(node);
    }

  private:
    const std::unordered_map<std::string, std::string>& renameMap;
    std::uint64_t& declRenamed;
    std::uint64_t& refRenamed;
};

void SyntaxTreeRewriter::reset_rename_map() {
    renameMap.clear();
    renamedDeclarations = 0;
    renamedReferences = 0;
}

void SyntaxTreeRewriter::set_prefix(rust::Str value) { prefix = std::string(value.data(), value.size()); }

void SyntaxTreeRewriter::set_suffix(rust::Str value) { suffix = std::string(value.data(), value.size()); }

void SyntaxTreeRewriter::set_excludes(const rust::Vec<rust::String> values) {
    excludes.clear();
    for (const auto& value : values) {
        excludes.insert(std::string(value));
    }
}

void SyntaxTreeRewriter::register_declarations(std::shared_ptr<SyntaxTree> tree) {
    if (prefix.empty() && suffix.empty()) {
        return;
    }

    std::unordered_set<std::string> declaredNames;
    DeclarationCollector collector(declaredNames);
    collector.visit(tree->root());

    for (const auto& name : declaredNames) {
        if (excludes.count(name)) {
            continue;
        }
        renameMap.insert_or_assign(name, prefix + name + suffix);
    }
}

std::shared_ptr<SyntaxTree> SyntaxTreeRewriter::rewrite_tree(std::shared_ptr<SyntaxTree> tree) {
    if (renameMap.empty()) {
        return tree;
    }

    std::uint64_t declRenamed = 0;
    std::uint64_t refRenamed = 0;
    MappedRewriter rewriter(renameMap, declRenamed, refRenamed);
    auto transformed = rewriter.transform(tree);
    renamedDeclarations += declRenamed;
    renamedReferences += refRenamed;
    return transformed;
}

// Print the given syntax tree with specified options
rust::String print_tree(const shared_ptr<SyntaxTree> tree, SlangPrintOpts options) {

    // Set up the printer with options
    SyntaxPrinter printer(tree->sourceManager());

    printer.setIncludeDirectives(options.include_directives);
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

    // Build a mapping from declared symbol names to the index of the tree that
    // declares them
    std::unordered_map<std::string_view, size_t> nameToTreeIndex;
    for (size_t i = 0; i < treeVec.size(); ++i) {
        const auto& metadata = treeVec[i]->getMetadata();
        for (auto name : metadata.getDeclaredSymbols()) {
            nameToTreeIndex.emplace(name, i);
        }
    }

    // Build a dependency graph where each tree points to the trees that declare
    // symbols it references
    std::vector<std::vector<size_t>> deps(treeVec.size());
    for (size_t i = 0; i < treeVec.size(); ++i) {
        const auto& metadata = treeVec[i]->getMetadata();
        std::unordered_set<size_t> seen;
        for (auto ref : metadata.getReferencedSymbols()) {
            auto it = nameToTreeIndex.find(ref);
            // Avoid duplicate dependencies in case of multiple references to the same
            // symbol
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

std::uint64_t renamed_declarations(const SyntaxTreeRewriter& rewriter) { return rewriter.renamed_declarations(); }

std::uint64_t renamed_references(const SyntaxTreeRewriter& rewriter) { return rewriter.renamed_references(); }
