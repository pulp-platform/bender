#include "slang_bridge.h"
#include "bender-slang/src/lib.rs.h" // Import the generated C++ definition of the structs

#include "slang/driver/Driver.h"
#include "slang/syntax/SyntaxPrinter.h"
#include "slang/syntax/SyntaxTree.h"

#include <stdexcept>
#include <vector>
#include <string>

using namespace slang;
using namespace slang::driver;
using namespace slang::syntax;

rust::String pickle(rust::Vec<rust::String> sources,
                    rust::Vec<rust::String> include_dirs,
                    rust::Vec<rust::String> defines,
                    SlangPrintOpts options) {
    Driver driver;
    driver.addStandardArgs();

    // 1. Construct Arguments from SlangFiles
    std::vector<std::string> arg_strings;
    arg_strings.push_back("slang_tool");

    for (const auto& source : sources) {
        arg_strings.push_back(std::string(source));
    }
    for (const auto& path : include_dirs) {
        arg_strings.push_back("-I");
        arg_strings.push_back(std::string(path));
    }

    for (const auto& def : defines) {
        arg_strings.push_back("-D");
        arg_strings.push_back(std::string(def));
    }

    // Convert to C-style argv
    std::vector<const char*> c_args;
    c_args.reserve(arg_strings.size());
    for (const auto& s : arg_strings) c_args.push_back(s.c_str());

    // 2. Run Compilation
    if (!driver.parseCommandLine(c_args.size(), c_args.data())) {
        throw std::runtime_error("Failed to parse command line arguments.");
    }

    if (!driver.processOptions()) {
        throw std::runtime_error("Failed to process options.");
    }

    bool parseSuccess = driver.parseAllSources();
    bool diagSuccess = driver.reportDiagnostics(false);

    if (!parseSuccess || !diagSuccess) {
        throw std::runtime_error("Parsing failed. Check stderr for details.");
    }

    auto& syntaxTrees = driver.syntaxTrees;
    if (syntaxTrees.empty()) {
        return "";
    }

    // 3. Configure Printer from SlangPrinterOptions
    SyntaxPrinter printer(driver.sourceManager);

    printer.setIncludeDirectives(options.include_directives);
    printer.setExpandIncludes(options.expand_includes);
    printer.setExpandMacros(options.expand_macros);
    printer.setSquashNewlines(options.squash_newlines);
    printer.setIncludeComments(options.include_comments);

    for (auto& tree : syntaxTrees) {
        printer.print(*tree);
    }

    return rust::String(printer.str());
}
