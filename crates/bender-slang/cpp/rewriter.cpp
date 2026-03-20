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

namespace {
// Returns true for language-defined scoped roots that must never be renamed.
bool is_reserved_scope_root(string_view name) {
    return name == "$unit" || name == "local" || name == "super" || name == "this";
}
} // namespace

std::unique_ptr<SyntaxTreeRewriter> new_syntax_tree_rewriter() { return std::make_unique<SyntaxTreeRewriter>(); }

// Pass 1: collects declarations and renames declaration sites.
class DeclarationRewriter : public SyntaxRewriter<DeclarationRewriter> {
  public:
    DeclarationRewriter(std::unordered_map<std::string, std::string>& renameMap, const std::string& prefix,
                        const std::string& suffix, const std::unordered_set<std::string>& excludes,
                        std::uint64_t& declRenamed)
        : renameMap(renameMap), prefix(prefix), suffix(suffix), excludes(excludes), declRenamed(declRenamed) {}

    string_view declaration_name(string_view name) {
        if (prefix.empty() && suffix.empty()) {
            return {};
        }
        if (excludes.count(std::string(name))) {
            return {};
        }

        auto [it, inserted] = renameMap.try_emplace(std::string(name), prefix + std::string(name) + suffix);
        (void)inserted;
        return string_view(it->second);
    }

    // e.g.: "module top;" -> "module p_top_s;" and "endmodule : top" -> "endmodule : p_top_s".
    void handle(const ModuleDeclarationSyntax& node) {
        if (node.header->name.isMissing()) {
            visitDefault(node);
            return;
        }

        auto newName = declaration_name(node.header->name.valueText());
        if (newName.empty()) {
            visitDefault(node);
            return;
        }

        auto newNameToken = node.header->name.withRawText(alloc, newName);

        ModuleHeaderSyntax* newHeader = deepClone(*node.header, alloc);
        newHeader->name = newNameToken;

        replace(*node.header, *newHeader);
        declRenamed++;

        // Also rename the end label if present (e.g., `endmodule : module_name`).
        if (node.blockName && !node.blockName->name.isMissing()) {
            auto newBlockNameToken = node.blockName->name.withRawText(alloc, newName);
            NamedBlockClauseSyntax* newBlockName = deepClone(*node.blockName, alloc);
            newBlockName->name = newBlockNameToken;
            replace(*node.blockName, *newBlockName);
        }

        visitDefault(node);
    }

  private:
    std::unordered_map<std::string, std::string>& renameMap;
    const std::string& prefix;
    const std::string& suffix;
    const std::unordered_set<std::string>& excludes;
    std::uint64_t& declRenamed;
};

// Pass 2: rewrites references based on the map built in pass 1.
// Internally this is split into:
//  - 2a structural references (instantiations / imports / virtual interfaces)
//  - 2b scoped-name references
class ReferenceRewriter : public SyntaxRewriter<ReferenceRewriter> {
  public:
    ReferenceRewriter(const std::unordered_map<std::string, std::string>& renameMap, std::uint64_t& refRenamed)
        : renameMap(renameMap), refRenamed(refRenamed) {}

    string_view mapped_name(string_view name) const {
        auto it = renameMap.find(std::string(name));
        if (it == renameMap.end()) {
            return {};
        }
        return string_view(it->second);
    }

    // Returns the mapped replacement for the left side of a scoped name
    // (e.g. common_pkg in common_pkg::state_t), or empty if not renamable.
    string_view mapped_scoped_left_name(const ScopedNameSyntax& node) const {
        if (node.left->kind != SyntaxKind::IdentifierName) {
            return {};
        }

        auto& leftNode = node.left->as<IdentifierNameSyntax>();
        auto name = leftNode.identifier.valueText();
        if (is_reserved_scope_root(name)) {
            return {};
        }
        return mapped_name(name);
    }

    // e.g.: "core u_core();" -> "p_core_s u_core();".
    void handle(const HierarchyInstantiationSyntax& node) {
        if (node.type.kind != TokenKind::Identifier) {
            visitDefault(node);
            return;
        }

        auto newName = mapped_name(node.type.valueText());
        if (newName.empty()) {
            visitDefault(node);
            return;
        }

        auto newNameToken = node.type.withRawText(alloc, newName);
        HierarchyInstantiationSyntax* newNode = deepClone(node, alloc);
        newNode->type = newNameToken;

        // Preserve scoped renames in overridden parameters of this
        // instantiation, which would otherwise be shadowed by replacing
        // the whole instantiation node.
        rewrite_scoped_names_inplace(*newNode);

        replace(node, *newNode);
        refRenamed++;
    }

    // e.g.: "import common_pkg::*;" -> "import p_common_pkg_s::*;".
    void handle(const PackageImportItemSyntax& node) {
        if (node.package.isMissing()) {
            return;
        }

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
    }

    // e.g.: "virtual bus_intf v_if;" -> "virtual p_bus_intf_s v_if;".
    void handle(const VirtualInterfaceTypeSyntax& node) {
        if (node.name.isMissing()) {
            return;
        }

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
    }

    // e.g.: "common_pkg::state_t" -> "p_common_pkg_s::state_t".
    void handle(const ScopedNameSyntax& node) {
        auto newName = mapped_scoped_left_name(node);
        if (newName.empty()) {
            visitDefault(node);
            return;
        }

        auto& leftNode = node.left->as<IdentifierNameSyntax>();
        auto newNameToken = leftNode.identifier.withRawText(alloc, newName);

        IdentifierNameSyntax* newLeft = deepClone(leftNode, alloc);
        newLeft->identifier = newNameToken;

        ScopedNameSyntax* newNode = deepClone(node, alloc);
        newNode->left = newLeft;

        replace(node, *newNode);
        refRenamed++;
    }

  private:
    // Rewrites only the left identifier of a scoped name in-place if mapped.
    void rewrite_scoped_name_left(ScopedNameSyntax& node) {
        auto newName = mapped_scoped_left_name(node);
        if (newName.empty()) {
            return;
        }

        auto& leftNode = node.left->as<IdentifierNameSyntax>();
        leftNode.identifier = leftNode.identifier.withRawText(alloc, newName);
        refRenamed++;
    }

    // Walks a subtree and rewrites all scoped-name left identifiers in-place.
    // Used on cloned instantiation subtrees before replacing the parent node.
    void rewrite_scoped_names_inplace(SyntaxNode& root) {
        if (auto* scoped = root.as_if<ScopedNameSyntax>()) {
            rewrite_scoped_name_left(*scoped);
        }

        for (size_t i = 0; i < root.getChildCount(); i++) {
            if (auto* child = root.childNode(i)) {
                rewrite_scoped_names_inplace(*child);
            }
        }
    }

    const std::unordered_map<std::string, std::string>& renameMap;
    std::uint64_t& refRenamed;
};

void SyntaxTreeRewriter::set_prefix(rust::Str value) { prefix = std::string(value.data(), value.size()); }

void SyntaxTreeRewriter::set_suffix(rust::Str value) { suffix = std::string(value.data(), value.size()); }

void SyntaxTreeRewriter::set_excludes(const rust::Vec<rust::String> values) {
    excludes.clear();
    for (const auto& value : values) {
        excludes.insert(std::string(value));
    }
}

// Pass 1: collect declaration names and rename declaration sites.
std::shared_ptr<SyntaxTree> SyntaxTreeRewriter::rewrite_declarations(std::shared_ptr<SyntaxTree> tree) {
    if (prefix.empty() && suffix.empty()) {
        return tree;
    }

    std::uint64_t declRenamed = 0;
    DeclarationRewriter rewriter(renameMap, prefix, suffix, excludes, declRenamed);
    auto transformed = rewriter.transform(tree);
    renamedDeclarations += declRenamed;
    return transformed;
}

// Pass 2: rename references using the map built in pass 1.
std::shared_ptr<SyntaxTree> SyntaxTreeRewriter::rewrite_references(std::shared_ptr<SyntaxTree> tree) {
    if (renameMap.empty()) {
        return tree;
    }

    std::uint64_t refRenamed = 0;
    ReferenceRewriter rewriter(renameMap, refRenamed);
    auto transformed = rewriter.transform(tree);
    renamedReferences += refRenamed;
    return transformed;
}

std::uint64_t renamed_declarations(const SyntaxTreeRewriter& rewriter) { return rewriter.renamed_declarations(); }

std::uint64_t renamed_references(const SyntaxTreeRewriter& rewriter) { return rewriter.renamed_references(); }
