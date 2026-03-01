// Copyright (c) 2025 ETH Zurich
// Tim Fischer <fischeti@iis.ee.ethz.ch>

#include "bender-slang/src/lib.rs.h"
#include "slang/syntax/CSTSerializer.h"
#include "slang/syntax/SyntaxPrinter.h"
#include "slang/text/Json.h"
#include "slang_bridge.h"

using namespace slang;
using namespace slang::syntax;

using std::shared_ptr;

// Prints the given syntax tree back to SystemVerilog source code,
// with options to control the printing behavior
rust::String print_tree(const shared_ptr<SyntaxTree> tree, SlangPrintOpts options) {
    SyntaxPrinter printer(tree->sourceManager());

    printer.setIncludeDirectives(options.include_directives);
    printer.setExpandIncludes(true);
    printer.setExpandMacros(options.expand_macros);
    printer.setSquashNewlines(options.squash_newlines);
    printer.setIncludeComments(options.include_comments);

    printer.print(tree->root());
    return rust::String(printer.str());
}

// Dumps the given syntax tree to a JSON string for debugging/analysis purposes
rust::String dump_tree_json(std::shared_ptr<SyntaxTree> tree) {
    JsonWriter writer;
    writer.setPrettyPrint(true);

    CSTSerializer serializer(writer);
    serializer.serialize(*tree);

    return rust::String(std::string(writer.view()));
}
