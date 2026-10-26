//! EZCAD `markcfg` import (latin-1, `[LMC_CFG]`, case-preserving keys).

use std::collections::BTreeMap;

use openlaser_types::{Mm, MmPerS};

use crate::{Error, Kinematics, MachineProfile, validate_profile};

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
    let text: String = bytes.iter().map(|&b| char::from(b)).collect();
    let mut in_lmc = false;
    let mut found = false;
    let mut values = BTreeMap::new();
    for (line_no, raw) in text.lines().enumerate() {
        let line = raw.trim().trim_start_matches('\u{feff}');
        if line.is_empty() || line.starts_with(';') || line.starts_with('#') {
            continue;
        }
        if line.starts_with('[') && line.ends_with(']') {
            in_lmc = line == "[LMC_CFG]";
            found |= in_lmc;
            continue;
        }
        if !in_lmc {
            continue;
        }
        let Some((key, value)) = line.split_once('=') else {
            return Err(Error::InvalidMarkCfg(format!("line {} in [LMC_CFG] has no '='", line_no + 1)));
        };
        let key = key.trim();
        if key.is_empty() {
            return Err(Error::InvalidMarkCfg(format!("line {} has an empty key", line_no + 1)));
        }
        values.insert(key.to_owned(), value.trim().to_owned());
    }
    if !found {
        return Err(Error::InvalidMarkCfg("missing [LMC_CFG] section".to_owned()));
    }
    Ok(MarkCfg { values })
}

pub fn apply(cfg: &MarkCfg, base: &MachineProfile) -> Result<(MachineProfile, Applied), Error> {
    let mut profile = base.clone();
    let Kinematics::Galvo(galvo) = &mut profile.kind else {
        return Err(Error::InvalidProfile("markcfg can only be applied to a galvo profile".to_owned()));
    };
    let mut report = Applied::default();
    for (key, raw) in &cfg.values {
        let applied = match key.as_str() {
            "FIELDSIZE" => {
                let field = positive_f64(key, raw)?;
                galvo.encoding_field = Mm::new(field).map_err(|e| Error::InvalidMarkCfg(e.to_string()))?;
                let half = field / 2.0;
                galvo.work_area.min[0] = galvo.work_area.min[0].max(-half);
                galvo.work_area.min[1] = galvo.work_area.min[1].max(-half);
                galvo.work_area.max[0] = galvo.work_area.max[0].min(half);
                galvo.work_area.max[1] = galvo.work_area.max[1].min(half);
                true
            }
            "MINPWMFREQ" => {
                galvo.source.freq_khz[0] = positive_f64(key, raw)? / 1000.0;
                true
            }
            "MAXPWMFREQ" => {
                galvo.source.freq_khz[1] = positive_f64(key, raw)? / 1000.0;
                true
            }
            "PULSEWIDTHS" => {
                galvo.source.pulse_table_ns = pulse_widths(raw)?;
                true
            }
            "REDLIGHTSPEED" => {
                galvo.light_speed = MmPerS::new(positive_f64(key, raw)?).map_err(|e| Error::InvalidMarkCfg(e.to_string()))?;
                true
            }
            "INVERTX" => {
                galvo.cal.negate[0] = bool_value(key, raw)?;
                true
            }
            "INVERTY" => {
                galvo.cal.negate[1] = bool_value(key, raw)?;
                true
            }
            "SWAPXY" => {
                galvo.cal.swap_xy = bool_value(key, raw)?;
                true
            }
            "LASERONDELAY" => {
                galvo.timings.laser_on_us = finite_f64(key, raw)?;
                true
            }
            "LASEROFFDELAY" => {
                galvo.timings.laser_off_us = finite_f64(key, raw)?;
                true
            }
            "POLYGONDELAY" => {
                galvo.timings.polygon_us = finite_f64(key, raw)?;
                true
            }
            "JUMPDELAY" => {
                galvo.timings.jump_delay_long_us = finite_f64(key, raw)?;
                true
            }
            _ => false,
        };
        if applied {
            report.applied.push(key.clone());
        } else {
            report.ignored.push(key.clone());
        }
    }
    validate_profile(&profile).map_err(|items| Error::InvalidProfile(items.join("; ")))?;
    Ok((profile, report))
}

fn finite_f64(key: &str, raw: &str) -> Result<f64, Error> {
    let value = raw.parse::<f64>().map_err(|_| Error::InvalidMarkCfg(format!("{key} is not a number: `{raw}`")))?;
    if value.is_finite() { Ok(value) } else { Err(Error::InvalidMarkCfg(format!("{key} must be finite"))) }
}

fn positive_f64(key: &str, raw: &str) -> Result<f64, Error> {
    let value = finite_f64(key, raw)?;
    if value > 0.0 { Ok(value) } else { Err(Error::InvalidMarkCfg(format!("{key} must be positive"))) }
}

fn bool_value(key: &str, raw: &str) -> Result<bool, Error> {
    match raw.trim() {
        "0" => Ok(false),
        "1" => Ok(true),
        _ => Err(Error::InvalidMarkCfg(format!("{key} must be 0 or 1"))),
    }
}

fn pulse_widths(raw: &str) -> Result<Vec<u16>, Error> {
    let mut values = Vec::new();
    for part in raw.split(',') {
        let value = part.trim().parse::<u16>().map_err(|_| Error::InvalidMarkCfg(format!("invalid pulse width `{}`", part.trim())))?;
        if value == 0 {
            return Err(Error::InvalidMarkCfg("pulse widths must be positive".to_owned()));
        }
        values.push(value);
    }
    values.sort_unstable();
    values.dedup();
    if values.is_empty() {
        return Err(Error::InvalidMarkCfg("pulse-width table must not be empty".to_owned()));
    }
    Ok(values)
}
