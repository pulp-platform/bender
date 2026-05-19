// Copyright (c) 2026 ETH Zurich
// Alessandro Ottaviano <aottaviano@tenstorrent.com>

//! Parameter tracing utilities - handle struct field propagation and
//! parameter dataflow analysis.

/// Check if a parameter binding value references the given parameter.
///
/// This handles both direct parameter references and struct field accesses.
///
/// Examples:
/// - `param="SEP"`, `value="SEP"` → true (direct match)
/// - `param="Cfg"`, `value="Cfg.JTAG_BSR_ENABLE"` → true (struct field)
/// - `param="Cfg"`, `value="Cfg"` → true (whole struct)
/// - `param="SEP"`, `value="1'b1"` → false (literal, no reference)
/// - `param="NUM_CORES"`, `value="DTP_NUM_CORES"` → false (different param)
///
/// The matching is done by checking if the value:
/// 1. Exactly matches the parameter name, OR
/// 2. Starts with `param.` (struct field access)
pub fn value_references_param(param: &str, value: &str) -> bool {
    if value == param {
        return true;
    }

    // Check for word-boundary prefix: struct field "Cfg.field", or bare use "NUM_CORES + 2"
    if value.starts_with(param) {
        let rest = &value[param.len()..];
        // At a word boundary when followed by '.', end-of-string, or a non-identifier char.
        // This avoids false positives like param="Cfg" matching value="CfgOther".
        let at_boundary = rest.is_empty()
            || rest.starts_with('.')
            || !rest.starts_with(|c: char| c.is_alphanumeric() || c == '_');
        if at_boundary {
            return true;
        }
    }

    // Check if the parameter appears anywhere in a more complex expression
    // For example: "(Cfg.SMC_CPU_CONFIG == smc_pkg::SMC_4CORE)"
    // We want to detect that this expression uses "Cfg"
    value.contains(&format!("{}.", param)) || value.contains(&format!(" {}", param))
}

/// Check if a port binding expression references the given signal name.
///
/// Signal bindings are typically bare identifiers (e.g. `.clk_i(clk_smu_i)`),
/// so exact match covers most cases. Word-boundary matching catches the rare
/// cases where the signal appears inside an aggregate or conditional expression.
///
/// Examples:
/// - `signal="clk_smu_i"`, `expr="clk_smu_i"` → true (exact)
/// - `signal="clk_i"`,     `expr="clk_smu_i"` → false (substring, not a word)
/// - `signal="clk_smu_i"`, `expr="{a, clk_smu_i}"` → true (word boundary)
pub fn value_references_signal(signal: &str, expr: &str) -> bool {
    if expr == signal {
        return true;
    }
    // Check as a whole word inside an expression.  A "word boundary" here means
    // the signal is preceded/followed by a non-identifier character.
    let is_id_char = |c: char| c.is_alphanumeric() || c == '_';
    if let Some(pos) = expr.find(signal) {
        let before_ok = pos == 0 || !is_id_char(expr[..pos].chars().next_back().unwrap());
        let after_ok  = pos + signal.len() == expr.len()
            || !is_id_char(expr[pos + signal.len()..].chars().next().unwrap());
        if before_ok && after_ok {
            return true;
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_value_references_param_direct() {
        assert!(value_references_param("SEP", "SEP"));
        assert!(!value_references_param("SEP", "SEP_OTHER"));
        assert!(!value_references_param("SEP", "PRESEP"));
    }

    #[test]
    fn test_value_references_param_struct() {
        assert!(value_references_param("Cfg", "Cfg.JTAG_BSR_ENABLE"));
        assert!(value_references_param("Cfg", "Cfg"));
        assert!(!value_references_param("Cfg", "CfgOther"));
        assert!(!value_references_param("Cfg", "OtherCfg"));
    }

    #[test]
    fn test_value_references_param_expression() {
        assert!(value_references_param("Cfg", "(Cfg.SMC_CPU_CONFIG == smc_pkg::SMC_4CORE)"));
        assert!(value_references_param("NUM_CORES", "NUM_CORES + 2"));
    }

    #[test]
    fn test_value_references_signal_exact() {
        assert!(value_references_signal("clk_smu_i", "clk_smu_i"));
        assert!(!value_references_signal("clk_i", "clk_smu_i"));
        assert!(!value_references_signal("clk_smu_i", "clk_i"));
    }

    #[test]
    fn test_value_references_signal_word_boundary() {
        assert!(value_references_signal("clk_smu_i", "{a, clk_smu_i, b}"));
        assert!(!value_references_signal("clk_i", "{a, clk_smu_i, b}"));
    }

}
