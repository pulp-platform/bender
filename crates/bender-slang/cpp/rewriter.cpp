// Copyright (c) 2025 ETH Zurich
// Tim Fischer <fischeti@iis.ee.ethz.ch>

#include "slang/syntax/SyntaxVisitor.h"
#include "slang_bridge.h"

#include <unordered_map>
#include <unordered_set>

using namespace slang;
using namespace slang::syntax;
using namespace slang::parsing;

using std::string_view;

std::unique_ptr<SyntaxTreeRewriter> new_syntax_tree_rewriter() { return std::make_unique<SyntaxTreeRewriter>(); }

// A syntax visitor that collects the names of all declared modules/interfaces/packages in a syntax tree.
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

    // Returns the mapped name for the given name if it exists in the renameMap,
    // or an empty string_view otherwise.
    string_view mapped_name(string_view name) {
        auto it = renameMap.find(std::string(name));
        if (it == renameMap.end()) {
            return {};
        }
        return string_view(it->second);
    }

    // e.g.: "module top;" -> "module p_top_s;".
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

    // e.g.: "core u_core();" -> "p_core_s u_core();".
    void handle(const HierarchyInstantiationSyntax& node) {
        if (node.type.kind == TokenKind::Identifier) {
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

    // e.g.: "import common_pkg::*;" -> "import p_common_pkg_s::*;".
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

    // e.g.: "virtual bus_intf v_if;" -> "virtual p_bus_intf_s v_if;".
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

    // e.g.: "common_pkg::state_t" -> "p_common_pkg_s::state_t".
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

// Registers all declarations in the given syntax tree by adding entries to the renameMap.
void SyntaxTreeRewriter::register_declarations(std::shared_ptr<SyntaxTree> tree) {
    if (prefix.empty() && suffix.empty()) {
        return;
    }

    // Collect all declared symbol names in the tree.
    std::unordered_set<std::string> declaredNames;
    DeclarationCollector collector(declaredNames);
    collector.visit(tree->root());

    // Populate the renameMap with new names for all collected declarations,
    // except those in the excludes set.
    for (const auto& name : declaredNames) {
        if (excludes.count(name)) {
            continue;
        }
        renameMap.insert_or_assign(name, prefix + name + suffix);
    }
}

// Rewrites the given syntax tree by renaming declarations and references according to the renameMap.
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

std::uint64_t renamed_declarations(const SyntaxTreeRewriter& rewriter) { return rewriter.renamed_declarations(); }

std::uint64_t renamed_references(const SyntaxTreeRewriter& rewriter) { return rewriter.renamed_references(); }
