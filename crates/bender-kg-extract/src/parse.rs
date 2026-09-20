// Copyright (c) 2026 ETH Zurich
// Alessandro Ottaviano <aottaviano@tenstorrent.com>

//! Drive `bender-slang` over a sequence of source groups, mirroring the loop
//! in `bender pickle`.

use std::time::{Duration, Instant};

use bender_slang::SlangSession;
use sha2::{Digest, Sha256};

use crate::{Result, SourceGroupInput};

pub(crate) struct ParseOutcome {
    pub session: SlangSession,
    pub all_defines: Vec<String>,
    pub file_count: usize,
    pub srclist_hash: String,
    /// Per-group wall-clock duration of the corresponding `parse_group` call.
    /// Same length as the groups vec passed in. Used by callers to derive
    /// `slang_parse_max_group_s` for the build's phase summary.
    pub group_durations: Vec<Duration>,
}

/// Per-group parse: one `parse_group` call per Bender SourceGroup. Each group
/// gets its own preprocessor scope (incdirs + defines) just like pickle does.
/// When `single_unit` is set, slang inherits `\`define`s declared in earlier
/// groups so cross-package macro use (e.g. `\`AXI_TYPEDEF_ALL` defined in
/// `axi/typedef.svh` and used by `tt_noc2axi.sv` in another package) parses
/// without per-file `\`include`s. Mirrors vcs / `vlog -mfcu`.
/// When `lenient` is set, parse-time error diagnostics are reported but do
/// not abort the build (best-effort mode for repos with hostile inputs).
pub(crate) fn parse(
    groups: &[SourceGroupInput],
    single_unit: bool,
    lenient: bool,
) -> Result<ParseOutcome> {
    let mut session = SlangSession::new();
    session.set_single_unit(single_unit);
    session.set_lenient(lenient);
    let mut all_defines: Vec<String> = Vec::new();
    let mut file_count = 0usize;
    let mut hasher = Sha256::new();
    let mut group_durations: Vec<Duration> = Vec::with_capacity(groups.len());

    // Hash uses 3 bits to capture (single_unit, lenient) so that re-runs
    // with different parsing modes produce distinct srclist hashes.
    let mode_byte: u8 =
        (if single_unit { 0x02 } else { 0x01 }) | (if lenient { 0x10 } else { 0x00 });
    hasher.update([mode_byte]);
    for (i, group) in groups.iter().enumerate() {
        hasher.update(b"\x1e");
        for f in &group.files {
            hasher.update(f.as_bytes());
            hasher.update(b"\x1f");
        }
        hasher.update(b"\x1d");
        for d in &group.defines {
            hasher.update(d.as_bytes());
            hasher.update(b"\x1f");
            all_defines.push(d.clone());
        }
        hasher.update(b"\x1d");
        for inc in &group.include_dirs {
            hasher.update(inc.as_bytes());
            hasher.update(b"\x1f");
        }
        file_count += group.files.len();
        let t0 = Instant::now();
        session.parse_group(&group.files, &group.include_dirs, &group.defines)?;
        let dt = t0.elapsed();
        group_durations.push(dt);
        log::debug!(
            "parse_group #{i:03} files={} dt={:.3}s",
            group.files.len(),
            dt.as_secs_f64()
        );
    }

    Ok(ParseOutcome {
        session,
        all_defines,
        file_count,
        srclist_hash: hex_lower(&hasher.finalize()),
        group_durations,
    })
}

fn hex_lower(bytes: &[u8]) -> String {
    const HEX: &[u8] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len() * 2);
    for &b in bytes {
        out.push(HEX[(b >> 4) as usize] as char);
        out.push(HEX[(b & 0x0f) as usize] as char);
    }
    out
}
