//! What the operator reads before framing and before arming: the `EmissionSummary` decoded from the bytes that will
//! be sent (SPEC §1.3 rule 3), never the document. Also the S23 first-emission cap check. Names and messages in it
//! (layer names, notices, the machine id) are printed as plain, bounded text (`plain::field`): no escape sequence in
//! an agent-named layer can conceal or forge the numbers beside it.

use std::fmt::Write as _;
use std::time::Duration;

use openlaser_types::{Bounds, EmissionSummary, FrameReceipt, PassRow, Refusal, RefusalCode, SectionMode};

use crate::plain;

fn mode(m: SectionMode) -> &'static str {
    match m {
        SectionMode::Line => "Line",
        SectionMode::Fill => "Fill",
        SectionMode::Image => "Image",
    }
}

/// The layer key, then its name in quotes when it has one of its own (plain, at most `NAME_MAX_CHARS`, `"` shown as
/// `'`), so a name that reads like numbers stays visibly a name.
fn layer(r: &PassRow) -> String {
    let key = r.layer.to_string();
    let name = plain::field(&r.layer_name, plain::NAME_MAX_CHARS).replace('"', "'");
    if name.is_empty() || name == key { key } else { format!("{key} \"{name}\"") }
}

fn bounds(b: &Bounds) -> String {
    format!("({:.2}, {:.2}) to ({:.2}, {:.2}) mm", b.min[0], b.min[1], b.max[0], b.max[1])
}

fn secs(d: Duration) -> String {
    format!("{:.1} s", d.as_secs_f64())
}

fn yes_no(b: bool) -> &'static str {
    if b { "yes" } else { "no" }
}

/// The decoded summary as a table, plus repeats, notices, totals, the post-crash bound, machine and fingerprint.
pub fn text(s: &EmissionSummary) -> String {
    let mut out = String::new();
    if let Some(r) = &s.rotary {
        let _ = writeln!(out, "{r}");
    }
    let _ = writeln!(
        out,
        "Job {} on {} — every number below is decoded from the bytes that will be sent",
        s.fingerprint.short(),
        plain::field(&s.machine.0, plain::NAME_MAX_CHARS)
    );
    let _ = writeln!(
        out,
        "  {:<14} {:<5} {:>7} {:>9} {:>11} {:>7} {:>15} {:>6}  {:<38} {:>9}",
        "Layer", "Mode", "Pass", "Power", "Frequency", "Pulse", "Speed (actual)", "Paths", "Bounds", "Time"
    );
    for r in &s.rows {
        let freq = r.frequency_khz.map_or_else(|| "—".to_owned(), |k| format!("{k:.2} kHz"));
        let pulse = r.pulse_ns.map_or_else(|| "—".to_owned(), |n| format!("{n} ns"));
        let _ = writeln!(
            out,
            "  {:<14} {:<5} {:>7} {:>9} {:>11} {:>7} {:>15} {:>6}  {:<38} {:>9}",
            layer(r),
            mode(r.mode),
            format!("{}/{}", r.pass, r.of),
            format!("{:.2} %", r.power_pct),
            freq,
            pulse,
            format!("{:.1} mm/s", r.speed_mm_s),
            r.paths,
            bounds(&r.bounds),
            secs(r.estimate),
        );
    }
    let _ = writeln!(out, "Repeats: {}   Packets: {}   Total time: {}", s.repeats, s.packets, secs(s.estimate));
    let _ = writeln!(out, "Emission still possible after a host crash: up to {} (the E-stop and key switch stop the rest)", secs(s.crash_bound));
    let _ = writeln!(
        out,
        "Work area: {}   Reverse X: {}   Reverse Y: {}   Swap X/Y: {}",
        bounds(&s.work_area),
        yes_no(s.negate[0]),
        yes_no(s.negate[1]),
        yes_no(s.swap_xy)
    );
    for n in &s.notices {
        let _ = writeln!(out, "Notice {}: {}", n.n, plain::field(&n.message, plain::MESSAGE_MAX_CHARS));
    }
    out
}

/// S23: the first emission of a bench session refuses any row above the cap (non-finite power is never within it).
pub fn check_cap(s: &EmissionSummary, cap_pct: Option<f64>) -> Result<(), Refusal> {
    let Some(cap) = cap_pct else { return Ok(()) };
    let over: Vec<String> = s
        .rows
        .iter()
        .filter(|r| !(r.power_pct.is_finite() && cap.is_finite() && r.power_pct <= cap))
        .map(|r| format!("{} pass {}/{} at {:.2} %", layer(r), r.pass, r.of, r.power_pct))
        .collect();
    if over.is_empty() {
        return Ok(());
    }
    let mut refusal = Refusal::new(
        RefusalCode::BenchPowerCap,
        format!(
            "the first emission of a bench session is capped at {cap} %; above it: {}. Lower the power (built-ins: --power-pct) and run again",
            over.join(", ")
        ),
    );
    if let Some(r) = s.rows.iter().find(|r| !(r.power_pct.is_finite() && r.power_pct <= cap)) {
        refusal = refusal.layer(r.layer);
    }
    Err(refusal)
}

/// A receipt that lets Enter through: the job's fingerprint, ≥ 1 pass the board executed (S20), human-originated,
/// covering the job (not tool layers only).
pub fn receipt_accepts(r: &FrameReceipt, fp: &openlaser_types::Fingerprint) -> Result<(), Refusal> {
    if &r.fingerprint != fp {
        return Err(Refusal::new(RefusalCode::Stale, "the frame that finished belongs to another job; frame this job"));
    }
    if r.full_passes < 1 {
        return Err(Refusal::new(RefusalCode::FrameRequired, "no frame pass has been executed by the board yet; wait for one full pass"));
    }
    if r.origin != openlaser_types::FrameOrigin::Human {
        return Err(Refusal::new(RefusalCode::FrameNotHuman, "this frame was not started by a person"));
    }
    if !r.covers_job() {
        return Err(Refusal::new(RefusalCode::FrameDoesNotCoverJob, "this frame traced tool layers only, so it cannot clear the job for marking"));
    }
    Ok(())
}

#[cfg(test)]
pub(crate) mod tests {
    use openlaser_types::{Fingerprint, FrameOrigin, FrameSource, FrameStyle, LayerKey, MachineId, MonoTime, SessionId};

    use super::*;

    pub(crate) fn row(power_pct: f64) -> PassRow {
        PassRow {
            layer: LayerKey::Color(0),
            layer_name: "C00".into(),
            sub: 0,
            pass: 1,
            of: 1,
            mode: SectionMode::Line,
            power_pct,
            frequency_khz: Some(20_000.0 / 667.0),
            pulse_ns: Some(200),
            speed_mm_s: 276.0 * 236.68 * 1000.0 / 65_535.0,
            galvo: None,
            paths: 1,
            mark_mm: 40.0,
            bounds: Bounds { min: [-5.0, -5.0], max: [5.0, 5.0] },
            estimate: Duration::from_millis(62),
        }
    }

    pub(crate) fn summary(power_pct: f64) -> EmissionSummary {
        EmissionSummary {
            rotary: None,
            fingerprint: Fingerprint([1; 32]),
            bytes_hash: Fingerprint([2; 32]),
            machine: MachineId("jpt-m7-200-sim".into()),
            rows: vec![row(power_pct)],
            notices: Vec::new(),
            repeats: 1,
            estimate: Duration::from_millis(62),
            crash_bound: Duration::from_millis(62),
            packets: 1,
            work_area: Bounds { min: [-100.0, -100.0], max: [100.0, 100.0] },
            negate: [false, false],
            swap_xy: false,
        }
    }

    #[test]
    fn table_shows_the_decoded_values() {
        let t = text(&summary(2048.0 * 100.0 / 4095.0));
        for want in ["50.01 %", "29.99 kHz", "200 ns", "996.8 mm/s", "1/1", "(-5.00, -5.00) to (5.00, 5.00) mm", "jpt-m7-200-sim", "Repeats: 1"] {
            assert!(t.contains(want), "summary shows {want}:\n{t}");
        }
    }

    #[test]
    fn names_and_notices_are_plain_bounded_text() {
        // An agent-named layer that prints fake numbers, then conceals (ESC[8m) the real ones and everything after.
        let mut s = summary(80.0);
        if let Some(r) = s.rows.first_mut() {
            r.layer_name = "C00   Line  1/1  5.00 % (fake)\u{1b}[8m".into();
        }
        s.machine = MachineId("jpt\u{1b}[2J-m7".into());
        s.notices.push(openlaser_types::Notice {
            n: 1,
            code: openlaser_types::NoticeCode::OpenShapeUnhatched,
            message: format!("open shape\u{9b}8m\r\u{202E}{}", "x".repeat(400)),
            object: None,
            layer: None,
        });
        let t = text(&s);
        assert!(t.chars().all(|c| c == '\n' || !plain::hidden(c)), "{t:?}");
        assert!(t.contains("80.00 %"), "the real power is shown:\n{t}");
        assert!(t.contains("C00 \"C00   Line  1/1  5.00 % (fake)[…\""), "the name is quoted and bounded:\n{t}");
        assert!(t.contains("on jpt[2J-m7"), "{t}");
        let notice = t.lines().find(|l| l.starts_with("Notice 1: open shape8m")).unwrap_or_default();
        assert_eq!(notice.chars().count(), "Notice 1: ".len() + plain::MESSAGE_MAX_CHARS, "{notice}");
        // The refusal for a capped row names the layer the same way.
        let r = check_cap(&s, Some(10.0)).unwrap_err();
        assert!(!r.message.chars().any(plain::hidden), "{:?}", r.message);
    }

    #[test]
    fn cap_refuses_above_and_passes_at_or_below() {
        assert!(check_cap(&summary(10.0), Some(10.0)).is_ok());
        assert!(check_cap(&summary(50.0), None).is_ok());
        let r = check_cap(&summary(10.5), Some(10.0)).unwrap_err();
        assert_eq!(r.code, RefusalCode::BenchPowerCap);
        assert_eq!(r.layer, Some(LayerKey::Color(0)));
        assert_eq!(check_cap(&summary(f64::NAN), Some(10.0)).unwrap_err().code, RefusalCode::BenchPowerCap);
    }

    #[test]
    fn receipts_that_do_not_clear_enter() {
        let fp = Fingerprint([1; 32]);
        let ok = FrameReceipt {
            fingerprint: fp,
            machine: MachineId("m".into()),
            session: SessionId(1),
            full_passes: 1,
            at: MonoTime::default(),
            style: FrameStyle::Bounds,
            source: FrameSource::Job,
            origin: FrameOrigin::Human,
        };
        assert!(receipt_accepts(&ok, &fp).is_ok());
        assert_eq!(receipt_accepts(&ok, &Fingerprint([9; 32])).unwrap_err().code, RefusalCode::Stale);
        assert_eq!(receipt_accepts(&FrameReceipt { full_passes: 0, ..ok.clone() }, &fp).unwrap_err().code, RefusalCode::FrameRequired);
        assert_eq!(receipt_accepts(&FrameReceipt { origin: FrameOrigin::Agent, ..ok.clone() }, &fp).unwrap_err().code, RefusalCode::FrameNotHuman);
        assert_eq!(
            receipt_accepts(&FrameReceipt { source: FrameSource::ToolLayersOnly, ..ok.clone() }, &fp).unwrap_err().code,
            RefusalCode::FrameDoesNotCoverJob
        );
    }
}
