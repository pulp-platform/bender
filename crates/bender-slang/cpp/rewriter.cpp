// Copyright (c) 2025 ETH Zurich
// Tim Fischer <fischeti@iis.ee.ethz.ch>

#include "slang/syntax/SyntaxRewriter.h"
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

// Base for our rewriters. Every rename we perform is a single identifier token.
template <typename TDerived> class TokenRewriter : public SyntaxRewriter<TDerived> {
  protected:
    using SyntaxRewriter<TDerived>::alloc;
    using SyntaxRewriter<TDerived>::replaceToken;

    // Queues a rename of `tok`, which must be a direct token child of `owner`.
    // Trivia and source location are carried over from the original token.
    // Returns false if the token isn't a child of `owner`.
    bool rename_token(const SyntaxNode& owner, const Token& tok, string_view newName) {
        for (size_t i = 0, n = owner.getChildCount(); i < n; i++) {
            if (owner.childNode(i)) {
                continue;
            }
            // Non-missing tokens within one node have distinct locations, so
            // this identifies the child slot holding `tok`.
            auto child = owner.childToken(i);
            if (child && child.kind == tok.kind && child.location() == tok.location()) {
                replaceToken(owner, i, tok.withRawText(alloc, newName));
                return true;
            }
        }
        return false;
    }
};

std::unique_ptr<SyntaxTreeRewriter> new_syntax_tree_rewriter() { return std::make_unique<SyntaxTreeRewriter>(); }

// Pass 1: collects declarations and renames declaration sites.
class DeclarationRewriter : public TokenRewriter<DeclarationRewriter> {
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

        rename_token(*node.header, node.header->name, newName);
        declRenamed++;

        // Also rename the end label if present (e.g., `endmodule : module_name`).
        if (node.blockName && !node.blockName->name.isMissing()) {
            rename_token(*node.blockName, node.blockName->name, newName);
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
class ReferenceRewriter : public TokenRewriter<ReferenceRewriter> {
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
    // visitDefault still descends into the parameter overrides and instance
    // bodies, so scoped names nested in them are rewritten as usual.
    void handle(const HierarchyInstantiationSyntax& node) {
        if (node.type.kind == TokenKind::Identifier) {
            auto newName = mapped_name(node.type.valueText());
            if (!newName.empty() && rename_token(node, node.type, newName)) {
                refRenamed++;
            }
        }
        visitDefault(node);
    }

    // e.g.: "import common_pkg::*;" -> "import p_common_pkg_s::*;".
    void handle(const PackageImportItemSyntax& node) {
        if (!node.package.isMissing()) {
            auto newName = mapped_name(node.package.valueText());
            if (!newName.empty() && rename_token(node, node.package, newName)) {
                refRenamed++;
            }
        }
        visitDefault(node);
    }

    // e.g.: "virtual bus_intf v_if;" -> "virtual p_bus_intf_s v_if;".
    void handle(const VirtualInterfaceTypeSyntax& node) {
        if (!node.name.isMissing()) {
            auto newName = mapped_name(node.name.valueText());
            if (!newName.empty() && rename_token(node, node.name, newName)) {
                refRenamed++;
            }
        }
        visitDefault(node);
    }

    // e.g.: "common_pkg::state_t" -> "p_common_pkg_s::state_t".
    void handle(const ScopedNameSyntax& node) {
        auto newName = mapped_scoped_left_name(node);
        if (!newName.empty()) {
            auto& leftNode = node.left->as<IdentifierNameSyntax>();
            if (rename_token(leftNode, leftNode.identifier, newName)) {
                refRenamed++;
            }
        }
        visitDefault(node);
    }

  private:
    const std::unordered_map<std::string, std::string>& renameMap;
    std::uint64_t& refRenamed;
};

void SyntaxTreeRewriter::set_prefix(rust::Str value) { prefix = std::string(value.data(), value.size()); }

void SyntaxTreeRewriter::set_suffix(rust::Str value) { suffix = std::string(value.data(), value.size()); }

void SyntaxTreeRewriter::set_excludes(rust::Slice<const rust::String> values) {
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
