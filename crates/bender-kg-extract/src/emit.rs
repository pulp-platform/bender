// Copyright (c) 2026 ETH Zurich
// Alessandro Ottaviano <aottaviano@tenstorrent.com>

//! IR sinks (streaming + in-memory) and manifest assembly.

use std::io::Write;

use bender_kg_models::{IrRecord, Manifest, ModuleData};

use crate::{ExtractError, Result};

/// Streaming output target.
pub trait IrSink {
    fn emit(&mut self, rec: &IrRecord) -> Result<()>;
}

impl<W: Write> IrSink for W {
    fn emit(&mut self, rec: &IrRecord) -> Result<()> {
        bender_kg_models::write_ir_record(self, rec).map_err(ExtractError::Models)
    }
}

/// In-memory sink, useful for tests and the core's typed API.
#[derive(Debug, Default)]
pub struct VecSink {
    pub records: Vec<IrRecord>,
}

impl IrSink for VecSink {
    fn emit(&mut self, rec: &IrRecord) -> Result<()> {
        self.records.push(rec.clone());
        Ok(())
    }
}

/// Build the manifest from per-module statistics + parse metadata.
pub(crate) fn build_manifest(
    identity: bender_kg_models::DesignIdentity,
    modules: &[ModuleData],
    file_count: usize,
    srclist_hash: String,
    warnings: Vec<String>,
) -> Manifest {
    let module_count = modules.iter().filter(|m| !m.is_package).count();
    let package_count = modules.iter().filter(|m| m.is_package).count();
    let edge_count: usize = modules.iter().map(|m| m.instantiations.len()).sum();

    let mut manifest = Manifest::new(identity);
    manifest.file_count = file_count;
    manifest.module_count = module_count;
    manifest.package_count = package_count;
    manifest.edge_count = edge_count;
    manifest.srclist_hash = srclist_hash;
    manifest.slang_version = Some(env!("CARGO_PKG_VERSION").to_string());
    manifest.created_at = current_timestamp();
    manifest.extraction_warnings = warnings;
    manifest
}

fn current_timestamp() -> Option<String> {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .ok()
        .map(|d| format!("unix:{}", d.as_secs()))
}

/// Stream a Manifest record followed by one Module record per module.
pub(crate) fn stream<S: IrSink>(
    sink: &mut S,
    manifest: &Manifest,
    modules: &[ModuleData],
) -> Result<()> {
    sink.emit(&IrRecord::Manifest(manifest.clone()))?;
    for m in modules {
        sink.emit(&IrRecord::Module(m.clone()))?;
    }
    Ok(())
}
