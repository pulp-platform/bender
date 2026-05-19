// Copyright (c) 2026 ETH Zurich
// Alessandro Ottaviano <aottaviano@tenstorrent.com>

//! Synthetic preprocessor defines, mirroring `bender script flist-plus`.
//!
//! `flist-plus` automatically emits `+define+TARGET_<UPPER>` for each active
//! target plus `+define+TARGET_FLIST`. Many manifests rely on this convention
//! (`\`ifdef TARGET_SIMULATION ...`). `bender sources` does not synthesize
//! them, so consumers that bypass `flist-plus` (pickle, kg parse) have to do
//! it themselves to stay in parity.

/// Emit `TARGET_<UPPER>` defines for each active target plus the bookkeeping
/// `TARGET_FLIST` flag, matching `flist-plus` output.
pub fn target_defines(targets: &[String]) -> Vec<String> {
    let mut out = Vec::with_capacity(targets.len() + 1);
    for t in targets {
        out.push(format!("TARGET_{}", t.to_uppercase()));
    }
    out.push("TARGET_FLIST".to_string());
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn emits_uppercased_target_defines() {
        let out = target_defines(&["sim".into(), "smc_chiplet".into()]);
        assert_eq!(
            out,
            vec!["TARGET_SIM", "TARGET_SMC_CHIPLET", "TARGET_FLIST"]
        );
    }

    #[test]
    fn empty_targets_still_emit_flist_marker() {
        assert_eq!(target_defines(&[]), vec!["TARGET_FLIST"]);
    }
}
