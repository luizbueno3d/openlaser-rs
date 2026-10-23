//! EZCAD `markcfg` import (latin-1, `[LMC_CFG]`, case-preserving keys). WP0 stubs.

use std::collections::BTreeMap;

use crate::{Error, MachineProfile};

#[derive(Debug, Clone, PartialEq, Default)]
pub struct MarkCfg {
    pub values: BTreeMap<String, String>,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct Applied {
    pub applied: Vec<String>,
    pub ignored: Vec<String>,
}

/// latin-1, [LMC_CFG], case-preserving keys.
pub fn parse(bytes: &[u8]) -> Result<MarkCfg, Error> {
    let _ = bytes;
    Err(Error::NotImplemented("openlaser_machine::markcfg::parse"))
}

pub fn apply(cfg: &MarkCfg, base: &MachineProfile) -> Result<(MachineProfile, Applied), Error> {
    let _ = (cfg, base);
    Err(Error::NotImplemented("openlaser_machine::markcfg::apply"))
}
