// Copyright (c) 2025 ETH Zurich
// Tim Fischer <fischeti@iis.ee.ethz.ch>

#include "slang_bridge.h"

#include <cstdio>
#include <filesystem>
#include <stdexcept>

using namespace slang;
using namespace slang::syntax;

using std::shared_ptr;
using std::string;
using std::string_view;

std::unique_ptr<SlangSession> new_slang_session() { return std::make_unique<SlangSession>(); }

SlangContext::SlangContext(SourceManager& sm)
    : sourceManager(sm), diagEngine(sourceManager),
      diagClient(std::make_shared<TextDiagnosticClient>()) {
    diagEngine.addClient(diagClient);
}

void SlangContext::set_includes(const rust::Vec<rust::String>& incs) {
    for (const auto& inc : incs) {
        std::string incStr(inc.data(), inc.size());
        if (auto ec = sourceManager.addUserDirectories(incStr); ec) {
            throw std::runtime_error("Failed to add include directory '" + incStr + "': " + ec.message());
        }
    }
}

void SlangContext::set_defines(const rust::Vec<rust::String>& defs) {
    ppOptions.predefines.reserve(defs.size());
    for (const auto& def : defs) {
        ppOptions.predefines.emplace_back(def.data(), def.size());
    }
}

// Parses a list of source files and returns the resulting syntax trees as a vector (of shared pointers).
// All files in the group are parsed into a single SystemVerilog compilation
// unit so that `\`define`s declared in earlier files are visible in later
// ones. This matches how downstream simulators (VCS, Verilator, ...) treat
// the files of a Bender source group when invoked on a single command line.
// When `inherited` is non-empty, the listed macros are predefined into this
// group's preprocessor (used to propagate `define`s from prior groups).
// If parsing fails, an exception is thrown with the error message(s) from the
// diagnostic engine.
std::vector<std::shared_ptr<SyntaxTree>>
SlangContext::parse_files(const rust::Vec<rust::String>& paths,
                          SyntaxTree::MacroList inherited,
                          bool lenient) {
    Bag options;
    options.set(ppOptions);

    if (paths.empty()) {
        return {};
    }

    std::shared_ptr<SyntaxTree> tree;
    if (inherited.empty()) {
        // Fast path: load files directly via slang's path-span overload.
        std::vector<std::string> path_storage;
        path_storage.reserve(paths.size());
        std::vector<string_view> path_views;
        path_views.reserve(paths.size());
        for (const auto& p : paths) {
            path_storage.emplace_back(p.data(), p.size());
            path_views.emplace_back(path_storage.back());
        }

        auto result = SyntaxTree::fromFiles(path_views, sourceManager, options);
        if (!result) {
            auto& err = result.error();
            std::string msg = "System Error loading '" + std::string(err.second) +
                              "': " + err.first.message();
            throw std::runtime_error(msg);
        }
        tree = *result;
    } else {
        // Single-unit path: read each file into a SourceBuffer and feed them
        // to fromBuffers along with the macros inherited from prior groups.
        std::vector<slang::SourceBuffer> buffers;
        buffers.reserve(paths.size());
        for (const auto& p : paths) {
            std::filesystem::path fpath(std::string(p.data(), p.size()));
            auto buf = sourceManager.readSource(fpath, /*library=*/nullptr);
            if (!buf) {
                std::string msg = "System Error loading '" + fpath.string() +
                                  "': " + buf.error().message();
                throw std::runtime_error(msg);
            }
            buffers.push_back(*buf);
        }
        tree = SyntaxTree::fromBuffers(buffers, sourceManager, options, inherited);
    }

    diagClient->clear();
    diagEngine.clearIncludeStack();

    bool hasErrors = false;
    std::size_t errorCount = 0;
    for (const auto& diag : tree->diagnostics()) {
        if (diag.isError()) {
            hasErrors = true;
            ++errorCount;
        }
        diagEngine.issue(diag);
    }

    if (hasErrors && !lenient) {
        std::string rendered = diagClient->getString();
        if (rendered.empty()) {
            rendered = "Failed to parse source group";
        }
        throw std::runtime_error(rendered);
    }

    // Lenient mode: a parse error inside slang frequently leaves dangling /
    // null pointers in the syntax tree (e.g. missing declarator, header,
    // generate-block clauses) which the kg walker dereferences without null
    // checks. To keep clean files in the same group available to the walker,
    // we fall back to per-file parsing here: each file is parsed independently
    // (accumulating `\`define`s from the prior good files in the group so the
    // single-unit semantics still hold for clean files), the files that error
    // are dropped, the others are kept. This matches pyslang's "best effort"
    // result more closely than dropping the whole group, which previously
    // hid healthy modules like ws-tensix's `tt_tensix_with_l1` because a
    // sibling file in the same Bender entry pulled in an encrypted include.
    if (hasErrors && lenient) {
        std::vector<std::shared_ptr<SyntaxTree>> kept;
        kept.reserve(paths.size());

        // Per-file accumulator seeded with the macros inherited from prior
        // groups (so cross-group `--single-unit` propagation is preserved
        // even when a group falls back to per-file parsing).
        std::vector<const slang::syntax::DefineDirectiveSyntax*> intraGroupMacros(
            inherited.begin(), inherited.end());

        std::size_t droppedFiles = 0;
        for (const auto& p : paths) {
            std::filesystem::path fpath(std::string(p.data(), p.size()));
            auto buf = sourceManager.readSource(fpath, /*library=*/nullptr);
            if (!buf) {
                std::fprintf(stderr,
                             "[bender-slang] lenient: skipping unreadable file '%s': %s\n",
                             fpath.string().c_str(), buf.error().message().c_str());
                ++droppedFiles;
                continue;
            }

            SyntaxTree::MacroList macros(intraGroupMacros.data(), intraGroupMacros.size());
            std::vector<slang::SourceBuffer> oneBuffer{*buf};
            auto oneTree = SyntaxTree::fromBuffers(oneBuffer, sourceManager, options, macros);

            bool fileHasErrors = false;
            std::size_t fileErrors = 0;
            for (const auto& diag : oneTree->diagnostics()) {
                if (diag.isError()) {
                    fileHasErrors = true;
                    ++fileErrors;
                }
            }

            if (fileHasErrors) {
                std::fprintf(stderr,
                             "[bender-slang] lenient: dropping file '%s' (%zu parse error(s))\n",
                             fpath.string().c_str(), fileErrors);
                ++droppedFiles;
                continue;
            }

            auto fresh = oneTree->getDefinedMacros();
            intraGroupMacros.insert(intraGroupMacros.end(), fresh.begin(), fresh.end());
            kept.push_back(oneTree);
        }

        std::fprintf(stderr,
                     "[bender-slang] lenient: source group had %zu group-level error(s); per-file fallback kept %zu/%zu file(s), dropped %zu\n",
                     errorCount, kept.size(), paths.size(), droppedFiles);
        return kept;
    }

    return { tree };
}

// Parses a group of files with the given include paths and preprocessor defines.
// Stores the resulting syntax trees and contexts in the session for later retrieval and analysis.
// In single-unit mode, macros defined by prior groups are predefined into this
// group's preprocessor and the macros newly defined here are appended to the
// session's accumulator so the next group can inherit them too.
void SlangSession::parse_group(const rust::Vec<rust::String>& files, const rust::Vec<rust::String>& includes,
                               const rust::Vec<rust::String>& defines) {
    // The SourceManager is shared across groups (required for cross-group
    // elaboration via slang::ast::Compilation). Predefines stay per-group.
    auto ctx = std::make_unique<SlangContext>(sourceManager);
    ctx->set_includes(includes);
    ctx->set_defines(defines);

    SyntaxTree::MacroList inherited{};
    if (singleUnit && !accumulatedMacros.empty()) {
        inherited = SyntaxTree::MacroList(accumulatedMacros.data(), accumulatedMacros.size());
    }
    auto parsed = ctx->parse_files(files, inherited, lenient);

    if (singleUnit) {
        // In lenient mode the failing files inside this group are dropped
        // file-by-file (see `parse_files`); the clean siblings still expose
        // their `\`define`s for subsequent groups via `getDefinedMacros`.
        for (const auto& tree : parsed) {
            auto fresh = tree->getDefinedMacros();
            accumulatedMacros.insert(accumulatedMacros.end(), fresh.begin(), fresh.end());
        }
    }

    allTrees.reserve(allTrees.size() + parsed.size());
    for (const auto& tree : parsed) {
        allTrees.push_back(tree);
    }

    contexts.push_back(std::move(ctx));
}

// Keep only the trees at the given indices and drop the rest. Indices outside
// the current `allTrees` range are silently skipped so callers can pass the
// raw output of `reachable_tree_indices` without bounds-checking. Runs in
// O(N+K): one pass over the index list and one move-assignment.
void SlangSession::retain_trees(const rust::Vec<std::uint32_t>& indices) {
    std::vector<std::shared_ptr<SyntaxTree>> kept;
    kept.reserve(indices.size());
    for (auto i : indices) {
        if (i < allTrees.size()) {
            kept.push_back(allTrees[i]);
        }
    }
    allTrees = std::move(kept);
}

// Returns the number of syntax trees currently stored in the session.
std::size_t tree_count(const SlangSession& session) { return session.trees().size(); }

// Returns the syntax tree at the given index in the session.
std::shared_ptr<SyntaxTree> tree_at(const SlangSession& session, std::size_t index) {
    if (index >= session.trees().size()) {
        throw std::runtime_error("Tree index out of bounds.");
    }
    return session.trees()[index];
}
