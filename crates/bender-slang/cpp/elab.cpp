// Copyright (c) 2026 ETH Zurich
// Alessandro Ottaviano <aottaviano@tenstorrent.com>

// Elaboration walker. Builds a slang::ast::Compilation from the session's
// parsed SyntaxTrees, forces elaboration from the requested top modules, and
// emits resolved per-instance parameter bindings and port widths.

#include "slang_bridge.h"

#include "slang/ast/ASTVisitor.h"
#include "slang/ast/Compilation.h"
#include "slang/ast/Scope.h"
#include "slang/ast/symbols/CompilationUnitSymbols.h"
#include "slang/ast/symbols/InstanceSymbols.h"
#include "slang/ast/symbols/ParameterSymbols.h"
#include "slang/ast/symbols/PortSymbols.h"
#include "slang/ast/symbols/VariableSymbols.h"
#include "slang/ast/types/AllTypes.h"
#include "slang/ast/types/Type.h"
#include "slang/numeric/ConstantValue.h"
#include "slang/util/Bag.h"

#include "bender-slang/src/lib.rs.h"

#include <stdexcept>
#include <string>
#include <vector>

using namespace slang;
using namespace slang::ast;

namespace {

// Stringify a (value or type) parameter's resolved binding. Defensive against
// uninitialized values which can show up if elaboration partially failed.
std::string param_value(const ParameterSymbolBase& p) {
    if (p.symbol.kind == SymbolKind::TypeParameter) {
        const auto& tp = p.symbol.as<TypeParameterSymbol>();
        try {
            return tp.getTypeAlias().toString();
        } catch (...) {
            return "type";
        }
    }
    if (p.symbol.kind == SymbolKind::Parameter) {
        const auto& vp = p.symbol.as<ParameterSymbol>();
        try {
            return vp.getValue().toString();
        } catch (...) {
            return "";
        }
    }
    return "";
}

// Recursively flatten a packed struct/union type. `prefix` accumulates the
// dot-joined field path; the empty string denotes the port type itself, in
// which case scalar leaves contribute nothing (the parent's `total` already
// covers them). Packed arrays / unpacked / opaque types are leaves with a
// well-defined `getBitWidth()` and emit a single entry. Non-Scope types
// (anything that is not a packed struct/union) terminate the recursion.
void flatten_type(const Type& ty, const std::string& prefix,
                  std::vector<KgKeyValue>& out) {
    const Type& canon = ty.getCanonicalType();
    const Scope* fields_scope = nullptr;
    if (canon.kind == SymbolKind::PackedStructType) {
        fields_scope = &canon.as<PackedStructType>();
    } else if (canon.kind == SymbolKind::PackedUnionType) {
        fields_scope = &canon.as<PackedUnionType>();
    }
    if (fields_scope) {
        for (const Symbol& m : fields_scope->members()) {
            if (m.kind != SymbolKind::Field) continue;
            const auto& f = m.as<FieldSymbol>();
            std::string child = prefix.empty()
                ? std::string(f.name)
                : prefix + "." + std::string(f.name);
            flatten_type(f.getType(), child, out);
        }
        return;
    }
    if (prefix.empty()) return; // top-level scalar: covered by `total`
    KgKeyValue kv;
    kv.key = rust::String(prefix);
    std::int64_t w = 0;
    try {
        w = static_cast<std::int64_t>(canon.getBitWidth());
    } catch (...) {}
    kv.value = rust::String(std::to_string(w));
    out.push_back(std::move(kv));
}

// Build a KgPortWidth for one port: a `total` from `getType().getBitWidth()`
// plus a dot-flattened breakdown across nested packed structs / unions. If
// the canonical port type is `T [N-1:0]` (a packed array of T), record
// `element_count = N` and recurse `flatten_type` over T into the element
// template fields. Only one level of array is unwrapped; deeper nesting
// stays opaque inside `element_total` / per-field totals.
KgPortWidth collect_port(const Symbol& port) {
    KgPortWidth pw;
    pw.name = rust::String(std::string(port.name));
    pw.total = 0;
    pw.element_count = 0;
    pw.element_total = 0;
    if (port.kind != SymbolKind::Port) return pw;
    try {
        const Type& t = port.as<PortSymbol>().getType();
        pw.total = static_cast<std::int64_t>(t.getBitWidth());
        std::vector<KgKeyValue> fields;
        flatten_type(t, "", fields);
        for (auto& kv : fields) pw.fields.push_back(std::move(kv));

        const Type& canon = t.getCanonicalType();
        if (canon.kind == SymbolKind::PackedArrayType) {
            const auto& arr = canon.as<PackedArrayType>();
            pw.element_count = static_cast<std::int64_t>(arr.range.width());
            pw.element_total = static_cast<std::int64_t>(arr.elementType.getBitWidth());
            std::vector<KgKeyValue> elem_fields;
            flatten_type(arr.elementType, "", elem_fields);
            for (auto& kv : elem_fields) pw.element_fields.push_back(std::move(kv));
        }
    } catch (...) {}
    return pw;
}

// Resolve the *defining module name* of the scope containing `inst`. The
// containing scope is typically an `InstanceBodySymbol`; for instances inside
// a generate block we walk up parent scopes until we find one. Returns "" for
// top-level instances.
std::string parent_module_name(const InstanceSymbol& inst) {
    const Scope* scope = inst.getParentScope();
    while (scope) {
        const Symbol& asSym = scope->asSymbol();
        if (asSym.kind == SymbolKind::InstanceBody) {
            return std::string(asSym.as<InstanceBodySymbol>().getDefinition().name);
        }
        if (asSym.kind == SymbolKind::Root) {
            return {};
        }
        scope = asSym.getParentScope();
    }
    return {};
}

struct ElabVisitor : public ASTVisitor<ElabVisitor, /*VisitStatements=*/false,
                                        /*VisitExpressions=*/false> {
    KgElabResult& out;
    explicit ElabVisitor(KgElabResult& o) : out(o) {}

    void handle(const InstanceSymbol& inst) {
        KgInstanceContext ctx;
        ctx.parent_module = rust::String(parent_module_name(inst));
        ctx.instance_name = rust::String(std::string(inst.name));
        ctx.child_module = rust::String(std::string(inst.getDefinition().name));

        const InstanceBodySymbol& body = inst.body;
        for (const ParameterSymbolBase* p : body.getParameters()) {
            KgKeyValue kv;
            kv.key = rust::String(std::string(p->symbol.name));
            kv.value = rust::String(param_value(*p));
            ctx.param_bindings.push_back(std::move(kv));
        }
        for (const Symbol* port : body.getPortList()) {
            ctx.port_widths.push_back(collect_port(*port));
        }
        out.contexts.push_back(std::move(ctx));
        visitDefault(inst);
    }
};

} // namespace

KgElabResult walk_elaborated(const SlangSession& session, const rust::Vec<rust::String>& tops) {
    KgElabResult out;

    // CompilationOptions::topModules stores std::string_view (non-owning), so
    // the underlying std::strings must outlive the Compilation. Keep them in a
    // pre-sized vector to avoid reallocation invalidating the views.
    std::vector<std::string> top_storage;
    top_storage.reserve(tops.size());
    CompilationOptions opts;
    for (const auto& t : tops) {
        top_storage.emplace_back(t.data(), t.size());
        opts.topModules.insert(top_storage.back());
    }
    Bag bag;
    bag.set(opts);
    Compilation comp(bag);

    for (const auto& tree : session.trees()) {
        try {
            comp.addSyntaxTree(tree);
        } catch (const std::exception& ex) {
            out.warnings.push_back(rust::String(std::string("addSyntaxTree: ") + ex.what()));
        }
    }

    const RootSymbol* root = nullptr;
    try {
        root = &comp.getRoot();
    } catch (const std::exception& ex) {
        out.warnings.push_back(rust::String(std::string("getRoot: ") + ex.what()));
        return out;
    }

    // Walk only the user-requested top instances. Root also exposes synthetic
    // InstanceSymbols for uninstantiated definitions (parent=Root, name="");
    // visiting those would flood the output with definition-level noise.
    ElabVisitor v(out);
    try {
        for (const InstanceSymbol* top : root->topInstances) {
            top->visit(v);
        }
    } catch (const std::exception& ex) {
        out.warnings.push_back(rust::String(std::string("visit: ") + ex.what()));
    }
    return out;
}
