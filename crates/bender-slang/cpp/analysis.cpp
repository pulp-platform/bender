// Copyright (c) 2025 ETH Zurich
// Tim Fischer <fischeti@iis.ee.ethz.ch>

#include "slang_bridge.h"

#include <functional>
#include <iostream>
#include <stdexcept>
#include <string_view>
#include <unordered_map>
#include <unordered_set>

using namespace slang;

rust::Vec<std::uint32_t> reachable_tree_indices(const SlangSession& session, const rust::Vec<rust::String>& tops) {
    const auto& treeVec = session.trees();

    // Build the name-to-tree-index map with last-wins semantics, emitting a warning
    // whenever a later definition overwrites an earlier one.
    std::unordered_map<std::string_view, size_t> nameToTreeIndex;
    for (size_t i = 0; i < treeVec.size(); ++i) {
        for (auto name : treeVec[i]->getMetadata().getDeclaredSymbols()) {
            auto [it, inserted] = nameToTreeIndex.emplace(name, i);
            if (!inserted) {
                std::cerr << "warning[BND]: Module '" << name << "' defined in "
                          << treeVec[i]->sourceManager().getRawFileName(treeVec[i]->getSourceBufferIds()[0])
                          << " overwrites previous definition in "
                          << treeVec[it->second]->sourceManager().getRawFileName(
                                 treeVec[it->second]->getSourceBufferIds()[0])
                          << ".\n";
                it->second = i;
            }
        }
    }

    // Build a dependency graph where each tree points to the trees that declare
    // symbols it references.
    std::vector<std::vector<size_t>> deps(treeVec.size());
    for (size_t i = 0; i < treeVec.size(); ++i) {
        const auto& metadata = treeVec[i]->getMetadata();
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
            throw std::runtime_error("Top module not found in any parsed source file: " + std::string(name));
        }
        startIndices.push_back(it->second);
    }

    // Perform a DFS from the top modules to find all reachable trees.
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

    rust::Vec<std::uint32_t> result;
    for (size_t i = 0; i < reachable.size(); ++i) {
        if (reachable[i]) {
            result.push_back(static_cast<std::uint32_t>(i));
        }
    }
    return result;
}
