// Copyright (c) 2026 ETH Zurich
// Alessandro Ottaviano <aottaviano@tenstorrent.com>

//! Convert `bender_slang::KgModule` records (syntactic walk output) into
//! `bender_kg_models::ModuleData`.

use bender_kg_models::{
    Direction, ImportInfo, InstantiationInfo, ModuleData, ParamInfo, ParamKind, PortInfo,
};
use bender_slang::{KgImport, KgInstance, KgModule, KgParam, KgPort};

pub(crate) fn convert_module(m: &KgModule, design_alias: &str) -> ModuleData {
    ModuleData {
        name: m.name.clone(),
        file_path: m.file_path.clone(),
        design: design_alias.to_string(),
        is_package: m.is_package,
        line_start: positive(m.line_start),
        line_end: positive(m.line_end),
        param_block_lines: pair(m.param_block_start, m.param_block_end),
        port_block_lines: pair(m.port_block_start, m.port_block_end),
        parameters: m.parameters.iter().map(convert_param).collect(),
        ports: m.ports.iter().map(convert_port).collect(),
        instantiations: m.instantiations.iter().map(convert_instance).collect(),
        imports: m.imports.iter().map(convert_import).collect(),
        includes: Vec::new(),
        exported_typedefs: Vec::new(),
        description: None,
    }
}

fn convert_param(p: &KgParam) -> ParamInfo {
    ParamInfo {
        name: p.name.clone(),
        kind: parse_param_kind(&p.kind),
        default_value: p.default_value.clone(),
        is_type_param: p.is_type_param,
    }
}

fn parse_param_kind(s: &str) -> ParamKind {
    match s {
        "int" => ParamKind::Int,
        "bit" => ParamKind::Bit,
        "type" => ParamKind::Type,
        "string" => ParamKind::String,
        _ => ParamKind::Other,
    }
}

fn convert_port(p: &KgPort) -> PortInfo {
    PortInfo {
        name: p.name.clone(),
        direction: parse_direction(&p.direction),
        type_str: p.type_str.clone(),
        width_expr: p.width_expr.clone(),
        bit_width: if p.bit_width >= 0 {
            Some(p.bit_width)
        } else {
            None
        },
        is_type_param: p.is_type_param,
        type_ref: None,
    }
}

fn parse_direction(s: &str) -> Direction {
    match s {
        "input" => Direction::Input,
        "output" => Direction::Output,
        "inout" => Direction::Inout,
        "ref" => Direction::Ref,
        _ => Direction::Input,
    }
}

fn convert_instance(i: &KgInstance) -> InstantiationInfo {
    let mut param_bindings = std::collections::BTreeMap::new();
    for kv in &i.param_bindings {
        param_bindings.insert(kv.key.clone(), kv.value.clone());
    }
    let mut port_bindings = std::collections::BTreeMap::new();
    for kv in &i.port_bindings {
        port_bindings.insert(kv.key.clone(), kv.value.clone());
    }
    InstantiationInfo {
        module_name: i.module_name.clone(),
        instance_name: i.instance_name.clone(),
        param_bindings,
        resolved_param_values: std::collections::BTreeMap::new(),
        port_bindings,
        resolved_port_widths: std::collections::BTreeMap::new(),
        condition: None,
        line_start: positive(i.line_start),
        line_end: positive(i.line_end),
    }
}

fn convert_import(im: &KgImport) -> ImportInfo {
    ImportInfo {
        package_name: im.package_name.clone(),
        is_wildcard: im.is_wildcard,
        specific_symbols: im.specific_symbols.clone(),
    }
}

fn positive(n: i64) -> Option<i64> {
    if n > 0 { Some(n) } else { None }
}
fn pair(a: i64, b: i64) -> Option<(i64, i64)> {
    if a > 0 && b > 0 { Some((a, b)) } else { None }
}
