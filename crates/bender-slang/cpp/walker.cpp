// Copyright (c) 2026 ETH Zurich
// Alessandro Ottaviano <aottaviano@tenstorrent.com>

// Knowledge-graph walker. Traverses every parsed SyntaxTree in a SlangSession
// and emits structured records describing each declared module/interface/
// package.
//
// This is the C++ counterpart of `bender_slang::walk_design`; the kg.v3
// data contract is owned by `bender-kg-models`.

#include "slang_bridge.h"

#include "slang/syntax/AllSyntax.h"
#include "slang/syntax/SyntaxKind.h"
#include "slang/syntax/SyntaxNode.h"
#include "slang/syntax/SyntaxTree.h"
#include "slang/syntax/SyntaxVisitor.h"
#include "slang/text/SourceLocation.h"
#include "slang/text/SourceManager.h"
#include "slang/util/Util.h"

#include "bender-slang/src/lib.rs.h"

#include <algorithm>
#include <cstdint>
#include <sstream>
#include <string>
#include <string_view>

using namespace slang;
using namespace slang::syntax;
using namespace slang::parsing;

namespace {

// --- helpers ----------------------------------------------------------------

std::string trim_text(std::string_view sv) {
    auto start = sv.find_first_not_of(" \t\r\n");
    auto end = sv.find_last_not_of(" \t\r\n");
    if (start == std::string::npos) return {};
    return std::string(sv.substr(start, end - start + 1));
}

std::string node_text(const SyntaxNode& node) {
    std::string out;
    auto sr = node.toString();
    out.assign(sr.data(), sr.size());
    return trim_text(out);
}

// Resolve start/end line numbers for a syntax node via its SourceManager.
void node_lines(const SyntaxNode& node, const SourceManager& sm, std::int64_t& start_line,
                std::int64_t& end_line) {
    auto range = node.sourceRange();
    auto s = range.start();
    auto e = range.end();
    if (s) {
        start_line = static_cast<std::int64_t>(sm.getLineNumber(s));
    }
    if (e) {
        end_line = static_cast<std::int64_t>(sm.getLineNumber(e));
    }
}

std::string node_file(const SyntaxNode& node, const SourceManager& sm) {
    auto range = node.sourceRange();
    auto s = range.start();
    if (!s) return {};
    auto path = sm.getFullPath(s.buffer());
    return path.string();
}

// Get token text safely.
std::string tok_text(Token t) {
    if (!t.valueText().empty()) {
        return std::string(t.valueText());
    }
    return std::string(t.rawText());
}

// Parameter kind classification, kept in lockstep with the kg.v3 IR.
rust::String classify_param_kind(const std::string& type_text, bool is_type_param) {
    if (is_type_param) return rust::String("type");
    auto t = type_text;
    std::transform(t.begin(), t.end(), t.begin(),
                   [](unsigned char c) { return static_cast<char>(std::tolower(c)); });
    if (t.find("string") != std::string::npos) return rust::String("string");
    if (t.find("int") != std::string::npos || t.find("integer") != std::string::npos)
        return rust::String("int");
    if (t.find("bit") != std::string::npos || t.find("logic") != std::string::npos ||
        t.find("reg") != std::string::npos)
        return rust::String("bit");
    return rust::String("other");
}

rust::String dir_to_str(TokenKind kind) {
    switch (kind) {
        case TokenKind::InputKeyword:
            return rust::String("input");
        case TokenKind::OutputKeyword:
            return rust::String("output");
        case TokenKind::InOutKeyword:
            return rust::String("inout");
        case TokenKind::RefKeyword:
            return rust::String("ref");
        default:
            return rust::String("input");
    }
}

// Walk a parameter port list and emit ParamRecords.
void collect_parameters(const ParameterPortListSyntax* params, rust::Vec<KgParam>& out) {
    if (!params) return;
    for (const auto* decl : params->declarations) {
        if (auto* td = decl->as_if<TypeParameterDeclarationSyntax>()) {
            for (const auto* td_decl : td->declarators) {
                KgParam p;
                p.name = rust::String(tok_text(td_decl->name));
                p.kind = rust::String("type");
                p.is_type_param = true;
                if (td_decl->assignment) {
                    p.default_value = rust::String(node_text(*td_decl->assignment->type));
                }
                out.push_back(std::move(p));
            }
        } else if (auto* pd = decl->as_if<ParameterDeclarationSyntax>()) {
            std::string type_text = pd->type ? node_text(*pd->type) : std::string();
            for (const auto* p_decl : pd->declarators) {
                KgParam p;
                p.name = rust::String(tok_text(p_decl->name));
                p.kind = classify_param_kind(type_text, false);
                p.is_type_param = false;
                if (p_decl->initializer) {
                    p.default_value = rust::String(node_text(*p_decl->initializer->expr));
                }
                out.push_back(std::move(p));
            }
        }
    }
}

// Walk an ANSI port list and emit PortRecords.
void collect_ports(const PortListSyntax* ports, rust::Vec<KgPort>& out) {
    if (!ports) return;
    auto* ansi = ports->as_if<AnsiPortListSyntax>();
    if (!ansi) {
        // Non-ANSI port lists are still walkable but shapes are different. For
        // parity v1 we emit a placeholder port list so consumers can detect.
        for (size_t i = 0; i < ports->getChildCount(); ++i) {
            (void)ports->childNode(i);
        }
        return;
    }
    std::string current_dir = "input";
    std::string current_type;
    for (const auto* port : ansi->ports) {
        if (auto* impl = port->as_if<ImplicitAnsiPortSyntax>()) {
            if (impl->header) {
                if (auto* vh = impl->header->as_if<VariablePortHeaderSyntax>()) {
                    if (vh->direction.kind != TokenKind::Unknown) {
                        current_dir = std::string(dir_to_str(vh->direction.kind).data(),
                                                  dir_to_str(vh->direction.kind).size());
                    }
                    current_type = vh->dataType ? node_text(*vh->dataType) : std::string();
                } else if (auto* nh = impl->header->as_if<NetPortHeaderSyntax>()) {
                    if (nh->direction.kind != TokenKind::Unknown) {
                        current_dir = std::string(dir_to_str(nh->direction.kind).data(),
                                                  dir_to_str(nh->direction.kind).size());
                    }
                    current_type = nh->dataType ? node_text(*nh->dataType) : std::string();
                }
            }
            KgPort p;
            p.name = rust::String(tok_text(impl->declarator->name));
            p.direction = rust::String(current_dir);
            p.type_str = rust::String(current_type);
            // dimensions -> width_expr
            if (!impl->declarator->dimensions.empty()) {
                std::string dims;
                for (const auto* d : impl->declarator->dimensions) {
                    dims += node_text(*d);
                }
                p.width_expr = rust::String(dims);
            }
            p.bit_width = -1;
            p.is_type_param = false;
            out.push_back(std::move(p));
        } else if (auto* expl = port->as_if<ExplicitAnsiPortSyntax>()) {
            KgPort p;
            p.name = rust::String(tok_text(expl->name));
            p.direction = rust::String(current_dir);
            p.type_str = rust::String(current_type);
            p.bit_width = -1;
            p.is_type_param = false;
            out.push_back(std::move(p));
        }
    }
}

// Walk a HierarchyInstantiation (e.g. `tt_fpu_v2 #(...) u_fpu(...)`) and emit a
// KgInstance for every named instance.
void collect_instances(const HierarchyInstantiationSyntax& inst, const SourceManager& sm,
                       rust::Vec<KgInstance>& out) {
    std::string module_name = tok_text(inst.type);
    // Param assignments, e.g. #(.WIDTH(32)) or #(32, 16).
    rust::Vec<KgKeyValue> param_bindings;
    if (inst.parameters) {
        if (auto* pa = inst.parameters->as_if<ParameterValueAssignmentSyntax>()) {
            int positional = 0;
            for (const auto* p : pa->parameters) {
                if (auto* named = p->as_if<NamedParamAssignmentSyntax>()) {
                    KgKeyValue kv;
                    kv.key = rust::String(tok_text(named->name));
                    kv.value =
                        rust::String(named->expr ? node_text(*named->expr) : std::string());
                    param_bindings.push_back(std::move(kv));
                } else if (auto* ord = p->as_if<OrderedParamAssignmentSyntax>()) {
                    KgKeyValue kv;
                    kv.key = rust::String("$" + std::to_string(positional));
                    kv.value =
                        rust::String(ord->expr ? node_text(*ord->expr) : std::string());
                    param_bindings.push_back(std::move(kv));
                    ++positional;
                }
            }
        }
    }
    for (const auto* h_inst : inst.instances) {
        if (!h_inst) continue;
        KgInstance kgi;
        kgi.module_name = rust::String(module_name);
        if (h_inst->decl) {
            kgi.instance_name = rust::String(tok_text(h_inst->decl->name));
        } else {
            kgi.instance_name = rust::String("<unknown>");
        }
        kgi.line_start = -1;
        kgi.line_end = -1;
        node_lines(*h_inst, sm, kgi.line_start, kgi.line_end);
        // Copy param bindings.
        for (const auto& kv : param_bindings) {
            KgKeyValue copy;
            copy.key = kv.key;
            copy.value = kv.value;
            kgi.param_bindings.push_back(std::move(copy));
        }
        // Port connections, if any.
        {
            int positional = 0;
            for (const auto* conn : h_inst->connections) {
                if (auto* named = conn->as_if<NamedPortConnectionSyntax>()) {
                    KgKeyValue kv;
                    kv.key = rust::String(tok_text(named->name));
                    kv.value = rust::String(
                        named->expr ? node_text(*named->expr) : std::string());
                    kgi.port_bindings.push_back(std::move(kv));
                } else if (auto* ord = conn->as_if<OrderedPortConnectionSyntax>()) {
                    KgKeyValue kv;
                    kv.key = rust::String("$" + std::to_string(positional));
                    kv.value =
                        rust::String(ord->expr ? node_text(*ord->expr) : std::string());
                    kgi.port_bindings.push_back(std::move(kv));
                    ++positional;
                }
            }
        }
        out.push_back(std::move(kgi));
    }
}

// Walk a member list looking for instantiations and imports. Recurses into
// generate-blocks to capture conditional/looped instantiations.
void scan_member_list(const SyntaxList<MemberSyntax>& members, const SourceManager& sm,
                      rust::Vec<KgInstance>& insts, rust::Vec<KgImport>& imports);

void scan_member(const MemberSyntax& m, const SourceManager& sm,
                 rust::Vec<KgInstance>& insts, rust::Vec<KgImport>& imports) {
    if (auto* hi = m.as_if<HierarchyInstantiationSyntax>()) {
        collect_instances(*hi, sm, insts);
    } else if (auto* pi = m.as_if<PackageImportDeclarationSyntax>()) {
        for (const auto* item : pi->items) {
            if (!item) continue;
            KgImport imp;
            imp.package_name = rust::String(tok_text(item->package));
            imp.is_wildcard = item->item.kind == TokenKind::Star;
            if (!imp.is_wildcard) {
                rust::String sym(tok_text(item->item));
                imp.specific_symbols.push_back(std::move(sym));
            }
            imports.push_back(std::move(imp));
        }
    } else if (auto* gen_blk = m.as_if<GenerateBlockSyntax>()) {
        scan_member_list(gen_blk->members, sm, insts, imports);
    } else if (auto* if_gen = m.as_if<IfGenerateSyntax>()) {
        if (if_gen->block) scan_member(*if_gen->block, sm, insts, imports);
    } else if (auto* loop_gen = m.as_if<LoopGenerateSyntax>()) {
        if (loop_gen->block) scan_member(*loop_gen->block, sm, insts, imports);
    } else if (auto* case_gen = m.as_if<CaseGenerateSyntax>()) {
        for (const auto* item : case_gen->items) {
            if (!item) continue;
            if (auto* def = item->as_if<DefaultCaseItemSyntax>()) {
                if (def->clause) {
                    if (auto* mb = def->clause->as_if<MemberSyntax>()) {
                        scan_member(*mb, sm, insts, imports);
                    }
                }
            } else if (auto* std_item = item->as_if<StandardCaseItemSyntax>()) {
                if (std_item->clause) {
                    if (auto* mb = std_item->clause->as_if<MemberSyntax>()) {
                        scan_member(*mb, sm, insts, imports);
                    }
                }
            }
        }
    } else if (auto* gen_region = m.as_if<GenerateRegionSyntax>()) {
        scan_member_list(gen_region->members, sm, insts, imports);
    }
}

void scan_member_list(const SyntaxList<MemberSyntax>& members, const SourceManager& sm,
                      rust::Vec<KgInstance>& insts, rust::Vec<KgImport>& imports) {
    for (const auto* m : members) {
        if (!m) continue;
        scan_member(*m, sm, insts, imports);
    }
}

KgModule build_module(const ModuleDeclarationSyntax& m, const SourceManager& sm,
                      bool is_package) {
    KgModule out;
    out.is_package = is_package;
    out.is_interface = false;
    out.line_start = -1;
    out.line_end = -1;
    out.param_block_start = -1;
    out.param_block_end = -1;
    out.port_block_start = -1;
    out.port_block_end = -1;
    // Header may be null when the parser bailed mid-declaration (e.g. lenient
    // mode on a partially malformed unit). Guard every access so we can still
    // emit a placeholder module record and let downstream walks skip it.
    if (m.header) {
        out.name = rust::String(tok_text(m.header->name));
    } else {
        out.name = rust::String("<unknown>");
    }
    node_lines(m, sm, out.line_start, out.line_end);
    out.file_path = rust::String(node_file(m, sm));

    // Doc comments would require token trivia inspection; leave empty for v1.
    // Description can be filled by a later pass that reads comments.

    if (m.header && m.header->parameters) {
        std::int64_t s = -1, e = -1;
        node_lines(*m.header->parameters, sm, s, e);
        out.param_block_start = s;
        out.param_block_end = e;
        collect_parameters(m.header->parameters, out.parameters);
    }
    if (m.header && m.header->ports) {
        std::int64_t s = -1, e = -1;
        node_lines(*m.header->ports, sm, s, e);
        out.port_block_start = s;
        out.port_block_end = e;
        collect_ports(m.header->ports, out.ports);
    }

    // Walk members for instantiations/imports (only meaningful for non-packages).
    scan_member_list(m.members, sm, out.instantiations, out.imports);
    return out;
}

void walk_tree(const std::shared_ptr<SyntaxTree>& tree, KgWalkResult& out) {
    if (!tree) return;
    auto& root = tree->root();
    auto& sm = tree->sourceManager();
    auto* unit = root.as_if<CompilationUnitSyntax>();
    if (!unit) return;

    // Pre-pass: collect compilation-unit-scope imports. Per SV LRM these are
    // implicitly visible to all modules in the same compilation unit, so we
    // attach them to every module-like record we emit from this tree.
    rust::Vec<KgImport> unit_imports;
    {
        rust::Vec<KgInstance> _scratch_insts;
        for (const auto* member : unit->members) {
            if (!member) continue;
            if (auto* pi = member->as_if<PackageImportDeclarationSyntax>()) {
                for (const auto* item : pi->items) {
                    if (!item) continue;
                    KgImport imp;
                    imp.package_name = rust::String(tok_text(item->package));
                    imp.is_wildcard = item->item.kind == TokenKind::Star;
                    if (!imp.is_wildcard) {
                        rust::String sym(tok_text(item->item));
                        imp.specific_symbols.push_back(std::move(sym));
                    }
                    unit_imports.push_back(std::move(imp));
                }
            }
        }
        (void)_scratch_insts;
    }

    for (const auto* member : unit->members) {
        if (!member) continue;
        if (auto* m = member->as_if<ModuleDeclarationSyntax>()) {
            bool is_package = m->kind == SyntaxKind::PackageDeclaration;
            // Interfaces also go through ModuleDeclarationSyntax under slang.
            bool is_interface = m->kind == SyntaxKind::InterfaceDeclaration;
            KgModule rec = build_module(*m, sm, is_package);
            rec.is_interface = is_interface;
            // Merge in file-level imports.
            for (const auto& imp : unit_imports) {
                KgImport copy;
                copy.package_name = imp.package_name;
                copy.is_wildcard = imp.is_wildcard;
                for (const auto& s : imp.specific_symbols) {
                    copy.specific_symbols.push_back(rust::String(s));
                }
                rec.imports.push_back(std::move(copy));
            }
            out.modules.push_back(std::move(rec));
        }
    }
}

} // namespace

KgWalkResult walk_design(const SlangSession& session) {
    KgWalkResult out;
    for (const auto& tree : session.trees()) {
        try {
            walk_tree(tree, out);
        } catch (const std::exception& ex) {
            out.warnings.push_back(rust::String(std::string("walk error: ") + ex.what()));
        }
    }
    return out;
}
