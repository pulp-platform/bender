// Copyright (c) 2025 ETH Zurich
// Tim Fischer <fischeti@iis.ee.ethz.ch>

#include "slang/diagnostics/PreprocessorDiags.h"
#include "slang_bridge.h"

#include <iostream>
#include <stdexcept>

using namespace slang;
using namespace slang::syntax;

using std::shared_ptr;
using std::string;
using std::string_view;

std::unique_ptr<SlangSession> new_slang_session() { return std::make_unique<SlangSession>(); }

SlangContext::SlangContext() : diagEngine(sourceManager), diagClient(std::make_shared<TextDiagnosticClient>()) {
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

// Parses a list of source files and returns one TreeEntry per file, each bundling the resulting
// syntax tree with the per-file facts slang reported (path, parse success, encryption).
// System-level errors (file unreadable, etc.) throw; per-file parse errors are surfaced
// non-fatally via the TreeEntry::parsedOk flag so the caller can apply policy.
std::vector<TreeEntry> SlangContext::parse_files(const rust::Vec<rust::String>& paths) {
    Bag options;
    options.set(ppOptions);

    std::vector<TreeEntry> out;
    out.reserve(paths.size());

    for (const auto& path : paths) {
        string_view pathView(path.data(), path.size());
        auto result = SyntaxTree::fromFile(pathView, sourceManager, options);

        // A system-level failure (file unreadable, etc.) is still fatal: the caller asked for
        // this path and we couldn't even open it. Parse errors are tolerated below.
        if (!result) {
            auto& err = result.error();
            std::string msg = "System Error loading '" + std::string(err.second) + "': " + err.first.message();
            throw std::runtime_error(msg);
        }

        auto tree = *result;
        diagClient->clear();
        diagEngine.clearIncludeStack();

        bool hasErrors = false;
        bool hasProtectDiag = false;
        for (const auto& diag : tree->diagnostics()) {
            hasErrors |= diag.isError();
            if (diag.code == slang::diag::ProtectedEnvelope) {
                hasProtectDiag = true;
            }
            diagEngine.issue(diag);
        }

        // Surface diagnostics for any file with errors, but keep going — the Rust side decides
        // what to do with the (possibly partial) tree. The encrypted flag lets the Rust side
        // discriminate IEEE-1735 encrypted IP (auto-tolerated) from real syntax bugs (fail by
        // default; tolerate with --allow-broken).
        if (hasErrors) {
            std::cerr << diagClient->getString();
        }

        out.push_back(TreeEntry{tree, std::string(path.data(), path.size()), !hasErrors, hasProtectDiag});
    }

    return out;
}

// Parses a group of files with the given include paths and preprocessor defines.
// Stores the resulting syntax trees and contexts in the session for later retrieval and analysis.
void SlangSession::parse_group(const rust::Vec<rust::String>& files, const rust::Vec<rust::String>& includes,
                               const rust::Vec<rust::String>& defines) {
    // Create a new context for this group of files.
    auto ctx = std::make_unique<SlangContext>();
    ctx->set_includes(includes);
    ctx->set_defines(defines);

    // Parse the files and append the resulting per-tree records to the session, so callers can
    // decide how to handle partially-parsed files.
    auto parsed = ctx->parse_files(files);
    treeEntries.reserve(treeEntries.size() + parsed.size());
    for (auto& entry : parsed) {
        treeEntries.push_back(std::move(entry));
    }

    contexts.push_back(std::move(ctx));
}
