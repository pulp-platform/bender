// Copyright (c) 2025 ETH Zurich
// Tim Fischer <fischeti@iis.ee.ethz.ch>

#include "bender-slang/src/lib.rs.h"
#include "slang/syntax/AllSyntax.h"
#include "slang_bridge.h"

#include <functional>
#include <iostream>
#include <memory>
#include <stdexcept>
#include <string_view>
#ifdef _WIN32
#include <io.h>
#else
#include <unistd.h>
#endif
#include <unordered_map>
#include <unordered_set>

using namespace slang;

static bool stderr_is_tty() {
#ifdef _WIN32
    return _isatty(_fileno(stderr)) != 0;
#else
    return isatty(STDERR_FILENO) != 0;
#endif
}

// Diagnostic for "a later top-level declaration shadows an earlier one with the same name".
// Lives in the General subsystem; the code is arbitrary but stable.
static const slang::DiagCode kDuplicateTopLevelDecl(slang::DiagSubsystem::General, 9999);
static constexpr std::string_view kDuplicateTopLevelDeclFormat = "module '{}' overwrites previous definition in '{}'";

// Converts an internal per-tree record into the cxx shared struct handed across the bridge.
static ParsedTree to_parsed(const TreeEntry& entry) {
    ParsedTree pt;
    pt.tree = entry.tree;
    pt.path = rust::String(entry.path);
    pt.parsed_ok = entry.parsedOk;
    pt.encrypted = entry.encrypted;
    return pt;
}

// Returns every parsed tree in the session, each bundled with its per-file facts. The order
// matches parse order across all parse_group calls.
rust::Vec<ParsedTree> all_trees(const SlangSession& session) {
    rust::Vec<ParsedTree> out;
    for (const auto& entry : session.entries()) {
        out.push_back(to_parsed(entry));
    }
    return out;
}

rust::Vec<ParsedTree> reachable_trees(const SlangSession& session, const rust::Vec<rust::String>& tops) {
    const auto& entries = session.entries();

    // One engine+client per distinct SourceManager. Each parse group creates its own
    // SourceManager (see SlangContext), so trees from different groups need different
    // engines; trees within a group share one.
    struct DiagState {
        std::unique_ptr<slang::DiagnosticEngine> engine;
        std::shared_ptr<slang::TextDiagnosticClient> client;
    };
    std::unordered_map<const slang::SourceManager*, DiagState> diagStates;
    const bool tty = stderr_is_tty();
    auto diagFor = [&](const slang::SourceManager& sm) -> DiagState& {
        auto [it, inserted] = diagStates.try_emplace(&sm);
        if (inserted) {
            it->second.engine = std::make_unique<slang::DiagnosticEngine>(sm);
            it->second.client = std::make_shared<slang::TextDiagnosticClient>();
            it->second.client->showColors(tty);
            it->second.engine->addClient(it->second.client);
            it->second.engine->setMessage(kDuplicateTopLevelDecl, std::string(kDuplicateTopLevelDeclFormat));
            it->second.engine->setSeverity(kDuplicateTopLevelDecl, slang::DiagnosticSeverity::Warning);
        }
        return it->second;
    };

    // Build the name-to-tree-index map with last-wins semantics, emitting a warning
    // whenever a later definition overwrites an earlier one.
    std::unordered_map<std::string_view, size_t> nameToTreeIndex;
    for (size_t i = 0; i < entries.size(); ++i) {
        const auto& metadata = entries[i].tree->getMetadata();

        auto checkAndInsert = [&](std::string_view name, slang::SourceLocation loc) {
            if (name.empty())
                return;
            auto [it, inserted] = nameToTreeIndex.emplace(name, i);
            if (inserted)
                return;

            const auto& prevBufferIds = entries[it->second].tree->getSourceBufferIds();
            std::string_view prevFile = prevBufferIds.empty()
                                            ? std::string_view("<unknown>")
                                            : entries[it->second].tree->sourceManager().getRawFileName(prevBufferIds[0]);

            auto& state = diagFor(entries[i].tree->sourceManager());
            slang::Diagnostic diag(kDuplicateTopLevelDecl, loc);
            diag << name;
            diag << prevFile;
            state.client->clear();
            state.engine->issue(diag);
            std::cerr << state.client->getString();
            it->second = i;
        };

        for (const auto& [decl, _] : metadata.nodeMeta)
            checkAndInsert(decl->header->name.valueText(), decl->header->name.location());
        for (const auto classDecl : metadata.classDecls)
            checkAndInsert(classDecl->name.valueText(), classDecl->name.location());
    }

    // Build a dependency graph where each tree points to the trees that declare
    // symbols it references.
    std::vector<std::vector<size_t>> deps(entries.size());
    for (size_t i = 0; i < entries.size(); ++i) {
        const auto& metadata = entries[i].tree->getMetadata();
        std::unordered_set<size_t> seen;
        for (auto ref : metadata.getReferencedSymbols()) {
            auto it = nameToTreeIndex.find(ref);
            // Avoid duplicate dependencies in case of multiple references to the same
            // symbol.
            if (it != nameToTreeIndex.end() && seen.insert(it->second).second) {
                deps[i].push_back(it->second);
            }
        }
    }

    // Map the top module names to their corresponding tree indices.
    std::vector<size_t> startIndices;
    startIndices.reserve(tops.size());
    for (const auto& top : tops) {
        std::string_view name(top.data(), top.size());
        auto it = nameToTreeIndex.find(name);
        if (it == nameToTreeIndex.end()) {
            throw std::runtime_error("Top module '" + std::string(name) + "' not found among " +
                                     std::to_string(nameToTreeIndex.size()) + " known top-level declarations.");
        }
        startIndices.push_back(it->second);
    }

    // Perform a DFS from the top modules to find all reachable trees.
    std::vector<bool> reachable(entries.size(), false);
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

    rust::Vec<ParsedTree> result;
    for (size_t i = 0; i < reachable.size(); ++i) {
        if (reachable[i]) {
            result.push_back(to_parsed(entries[i]));
        }
    }
    return result;
}

// Returns the deduped, canonical filesystem paths of every header file that was actually loaded
// via `include directives while parsing the given trees. Trees from different parse groups may
// live in different SourceManagers, so the lookup is per-tree.
rust::Vec<rust::String> resolved_include_paths_for(const rust::Vec<ParsedTree>& trees) {
    std::unordered_set<std::string> uniquePaths;
    for (const auto& parsed : trees) {
        const auto& tree = parsed.tree;
        if (!tree)
            continue;
        const auto& sm = tree->sourceManager();
        for (const auto& inc : tree->getIncludeDirectives()) {
            if (!inc.buffer.id.valid())
                continue;
            const auto& fullPath = sm.getFullPath(inc.buffer.id);
            if (!fullPath.empty()) {
                uniquePaths.insert(fullPath.string());
            }
        }
    }
    rust::Vec<rust::String> out;
    out.reserve(uniquePaths.size());
    for (const auto& p : uniquePaths) {
        out.push_back(rust::String(p));
    }
    return out;
}
