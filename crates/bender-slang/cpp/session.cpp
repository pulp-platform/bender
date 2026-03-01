// Copyright (c) 2025 ETH Zurich
// Tim Fischer <fischeti@iis.ee.ethz.ch>

#include "slang_bridge.h"

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

// Parses a list of source files and returns the resulting syntax trees as a vector (of shared pointers).
// If any file fails to parse, an exception is thrown with the error message(s) from the diagnostic engine.
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

// Parses a group of files with the given include paths and preprocessor defines.
// Stores the resulting syntax trees and contexts in the session for later retrieval and analysis.
void SlangSession::parse_group(const rust::Vec<rust::String>& files, const rust::Vec<rust::String>& includes,
                               const rust::Vec<rust::String>& defines) {
    // Create a new context for this group of files.
    auto ctx = std::make_unique<SlangContext>();
    ctx->set_includes(includes);
    ctx->set_defines(defines);

    // Parse the files and store the resulting syntax trees in the session.
    auto parsed = ctx->parse_files(files);
    allTrees.reserve(allTrees.size() + parsed.size());
    for (const auto& tree : parsed) {
        allTrees.push_back(tree);
    }

    contexts.push_back(std::move(ctx));
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
