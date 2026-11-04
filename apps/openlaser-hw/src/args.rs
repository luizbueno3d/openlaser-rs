//! The command line (SPEC §3.21). Unknown flags, missing values, repeated flags and malformed numbers are usage
//! errors (exit 2). Nothing here touches a device.

use std::path::PathBuf;

use openlaser_types::{FrameSource, FrameStyle, MachineId};

use crate::HwError;
use crate::builtin::Builtin;

pub const USAGE: &str = "\
usage:
  openlaser-hw detect                                       read-only enumeration (needs a `hw` build)
  openlaser-hw status  [--machine <id>] [--sim] [--trace <f>]
  openlaser-hw ports   [--watch] [--machine <id>] [--sim] [--trace <f>]
                                                            read the input port bits (foot switch, door); never writes
  openlaser-hw rotary  move --deg <d> | pos [--machine <id>] [--sim] [--trace <f>]
  openlaser-hw bench   start|status|end
  openlaser-hw profile show|set <key> <value>|import-om <calibration.json>|import-ezcad <markcfg> [--machine <id>]
  openlaser-hw frame   (<project> | --builtin <pattern>) [--loop] [--style bounds|contour] [--source job|tools|both]
                       [--extent <mm>] [--machine <id>] [--sim] [--trace <f>]
  openlaser-hw run     (<project> | --builtin <pattern>) [--layers <layer_settings.json>] [--repeats N] [--power-pct P]
                       [--allow-emission] [--machine <id>] [--sim] [--trace <f>]
patterns: dot | line | box10 | square50 | f-shape | corners
keys while the machine is active: Esc, Space or Ctrl-C stop it; Enter accepts the frame (run)
";

/// Where the design comes from.
#[derive(Debug, Clone, PartialEq)]
pub enum JobSource {
    Project(PathBuf),
    Builtin(Builtin),
}

/// Options shared by the device commands.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct DeviceOpts {
    pub machine: Option<MachineId>,
    pub sim: bool,
    pub trace: Option<PathBuf>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct StatusOpts {
    pub device: DeviceOpts,
}

/// `ports` (H11): read the LMC input port word, once or (`--watch`) until a stop key.
#[derive(Debug, Clone, PartialEq)]
pub struct PortsOpts {
    pub watch: bool,
    pub device: DeviceOpts,
}

#[derive(Debug, Clone, PartialEq)]
pub struct FrameOpts {
    pub job: JobSource,
    pub looping: bool,
    pub style: Option<FrameStyle>,
    pub source: Option<FrameSource>,
    /// Corners only: trace up to ±extent mm (red light only, H27).
    pub extent: Option<f64>,
    pub device: DeviceOpts,
}

#[derive(Debug, Clone, PartialEq)]
pub struct RunOpts {
    pub job: JobSource,
    pub layers: Option<PathBuf>,
    pub repeats: Option<u16>,
    /// Built-ins only: every row's power.
    pub power_pct: Option<f64>,
    pub allow_emission: bool,
    pub device: DeviceOpts,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BenchCmd {
    Start,
    Status,
    End,
}

#[derive(Debug, Clone, PartialEq)]
pub enum ProfileAction {
    Show,
    Set { key: String, value: String },
    ImportOm(PathBuf),
    ImportEzcad(PathBuf),
}

#[derive(Debug, Clone, PartialEq)]
pub struct ProfileOpts {
    pub action: ProfileAction,
    pub machine: Option<MachineId>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Cmd {
    Rotary { degrees: Option<f64>, device: DeviceOpts },
    Help,
    Detect,
    Status(StatusOpts),
    Ports(PortsOpts),
    Bench(BenchCmd),
    Profile(ProfileOpts),
    Frame(FrameOpts),
    Run(RunOpts),
}

fn usage(msg: impl Into<String>) -> HwError {
    HwError::Usage(msg.into())
}

/// Flags of one subcommand: `--name value` or bare `--name`; everything else is positional.
struct Flags<'a> {
    sub: &'a str,
    positional: Vec<&'a str>,
    values: Vec<(&'a str, Option<&'a str>)>,
}

/// `-0.5,-0.25` or `-50`: a negative number is a value (`profile set cal.offsetMm -0.5,-0.25`), never a flag.
fn negative_number(a: &str) -> bool {
    a.strip_prefix('-').is_some_and(|rest| rest.starts_with(|c: char| c.is_ascii_digit() || c == '.'))
}

impl<'a> Flags<'a> {
    /// `valued` flags take one value; `bare` flags take none. Anything else starting with `-` (other than a negative
    /// number) is a usage error, and so is a valued flag followed by another flag instead of its value.
    fn parse(sub: &'a str, args: &'a [String], valued: &[&str], bare: &[&str]) -> Result<Self, HwError> {
        let mut out = Flags { sub, positional: Vec::new(), values: Vec::new() };
        let mut it = args.iter();
        while let Some(a) = it.next() {
            let a = a.as_str();
            if a.starts_with('-') && a.len() > 1 && !negative_number(a) {
                if out.values.iter().any(|(n, _)| *n == a) {
                    return Err(usage(format!("`{sub}`: {a} given twice")));
                }
                if valued.contains(&a) {
                    let v = it.next().ok_or_else(|| usage(format!("`{sub}`: {a} needs a value")))?;
                    if v.starts_with("--") {
                        return Err(usage(format!("`{sub}`: {a} needs a value, got the flag {v}")));
                    }
                    out.values.push((a, Some(v.as_str())));
                } else if bare.contains(&a) {
                    out.values.push((a, None));
                } else {
                    return Err(usage(format!("`{sub}`: unknown flag {a}")));
                }
            } else {
                out.positional.push(a);
            }
        }
        Ok(out)
    }

    fn value(&self, name: &str) -> Option<&'a str> {
        self.values.iter().find(|(n, _)| *n == name).and_then(|(_, v)| *v)
    }

    fn has(&self, name: &str) -> bool {
        self.values.iter().any(|(n, _)| *n == name)
    }

    fn device(&self) -> DeviceOpts {
        DeviceOpts {
            machine: self.value("--machine").map(|m| MachineId(m.to_owned())),
            sim: self.has("--sim"),
            trace: self.value("--trace").map(PathBuf::from),
        }
    }

    fn job(&self) -> Result<JobSource, HwError> {
        let sub = self.sub;
        match (self.positional.as_slice(), self.value("--builtin")) {
            ([], Some(b)) => Ok(JobSource::Builtin(Builtin::parse(b).ok_or_else(|| usage(format!("`{sub}`: unknown pattern `{b}` ({})", Builtin::NAMES)))?)),
            ([p], None) => Ok(JobSource::Project(PathBuf::from(p))),
            ([], None) => Err(usage(format!("`{sub}` needs a project file or --builtin <pattern>"))),
            ([_], Some(_)) => Err(usage(format!("`{sub}` takes a project file or --builtin, not both"))),
            _ => Err(usage(format!("`{sub}` takes one project file"))),
        }
    }

    fn no_positional(&self) -> Result<(), HwError> {
        match self.positional.first() {
            None => Ok(()),
            Some(p) => Err(usage(format!("`{}`: unexpected argument `{p}`", self.sub))),
        }
    }
}

fn finite(sub: &str, name: &str, v: &str) -> Result<f64, HwError> {
    match v.trim().parse::<f64>() {
        Ok(x) if x.is_finite() => Ok(x),
        _ => Err(usage(format!("`{sub}`: {name} must be a finite number, got `{v}`"))),
    }
}

const DEVICE_VALUED: [&str; 2] = ["--machine", "--trace"];

pub fn parse(args: &[String]) -> Result<Cmd, HwError> {
    let Some((sub, rest)) = args.split_first() else { return Err(usage("no command given")) };
    match sub.as_str() {
        "help" | "--help" | "-h" => match rest.first() {
            None => Ok(Cmd::Help),
            Some(a) => Err(usage(format!("`help`: unexpected argument `{a}`"))),
        },
        "detect" => {
            let f = Flags::parse("detect", rest, &[], &[])?;
            f.no_positional()?;
            Ok(Cmd::Detect)
        }
        "status" => {
            let f = Flags::parse("status", rest, &DEVICE_VALUED, &["--sim"])?;
            f.no_positional()?;
            Ok(Cmd::Status(StatusOpts { device: f.device() }))
        }
        "ports" => {
            let f = Flags::parse("ports", rest, &DEVICE_VALUED, &["--sim", "--watch"])?;
            f.no_positional()?;
            Ok(Cmd::Ports(PortsOpts { watch: f.has("--watch"), device: f.device() }))
        }
        "bench" => match rest {
            [s] if s == "start" => Ok(Cmd::Bench(BenchCmd::Start)),
            [s] if s == "status" => Ok(Cmd::Bench(BenchCmd::Status)),
            [s] if s == "end" => Ok(Cmd::Bench(BenchCmd::End)),
            _ => Err(usage("`bench` takes exactly one of start, status, end")),
        },
        "rotary" => {
            let f = Flags::parse("rotary", rest, &["--deg", "--machine", "--trace"], &["--sim"])?;
            let degrees = match f.positional.as_slice() {
                ["move"] => Some(finite("rotary", "--deg", f.value("--deg").ok_or_else(|| usage("rotary move requires --deg"))?)?),
                ["pos"] if f.value("--deg").is_none() => None,
                _ => return Err(usage("rotary move --deg <d> | rotary pos")),
            };
            Ok(Cmd::Rotary { degrees, device: f.device() })
        }
        "profile" => parse_profile(rest),
        "frame" => parse_frame(rest),
        "run" => parse_run(rest),
        // The agent CLI's words, so a person typing them here learns where they are.
        other => Err(usage(format!("unknown command `{other}`"))),
    }
}

fn parse_profile(rest: &[String]) -> Result<Cmd, HwError> {
    let f = Flags::parse("profile", rest, &["--machine"], &[])?;
    let machine = f.value("--machine").map(|m| MachineId(m.to_owned()));
    let action = match f.positional.as_slice() {
        ["show"] => ProfileAction::Show,
        ["set", key, value] => ProfileAction::Set { key: (*key).to_owned(), value: (*value).to_owned() },
        ["import-om", path] => ProfileAction::ImportOm(PathBuf::from(path)),
        ["import-ezcad", path] => ProfileAction::ImportEzcad(PathBuf::from(path)),
        _ => return Err(usage("`profile` takes show | set <key> <value> | import-om <calibration.json> | import-ezcad <markcfg>")),
    };
    Ok(Cmd::Profile(ProfileOpts { action, machine }))
}

fn parse_frame(rest: &[String]) -> Result<Cmd, HwError> {
    let f = Flags::parse("frame", rest, &["--builtin", "--style", "--source", "--extent", "--machine", "--trace"], &["--loop", "--sim"])?;
    let job = f.job()?;
    let style = match f.value("--style") {
        None => None,
        Some("bounds") => Some(FrameStyle::Bounds),
        Some("contour") => Some(FrameStyle::Contour),
        Some(s) => return Err(usage(format!("`frame`: --style is bounds or contour, got `{s}`"))),
    };
    let source = match f.value("--source") {
        None => None,
        Some("job") => Some(FrameSource::Job),
        Some("tools") => Some(FrameSource::ToolLayersOnly),
        Some("both") => Some(FrameSource::JobAndToolLayers),
        Some(s) => return Err(usage(format!("`frame`: --source is job, tools or both, got `{s}`"))),
    };
    let extent = match f.value("--extent") {
        None => None,
        Some(v) => {
            if job != JobSource::Builtin(Builtin::Corners) {
                return Err(usage("`frame`: --extent is only for --builtin corners (H27)"));
            }
            let e = finite("frame", "--extent", v)?;
            if e <= 0.0 {
                return Err(usage(format!("`frame`: --extent must be above 0 mm, got {e}")));
            }
            Some(e)
        }
    };
    Ok(Cmd::Frame(FrameOpts { job, looping: f.has("--loop"), style, source, extent, device: f.device() }))
}

fn parse_run(rest: &[String]) -> Result<Cmd, HwError> {
    if rest.iter().any(|a| a == "--extent") {
        return Err(usage("`run` never accepts --extent: tracing past the work area is red light only (`frame --builtin corners --extent <mm>`)"));
    }
    let f = Flags::parse("run", rest, &["--builtin", "--layers", "--repeats", "--power-pct", "--machine", "--trace"], &["--allow-emission", "--sim"])?;
    let job = f.job()?;
    let builtin = matches!(job, JobSource::Builtin(_));
    let layers = f.value("--layers").map(PathBuf::from);
    if layers.is_some() && builtin {
        return Err(usage("`run`: --layers applies to an OM workspace.json, not to a built-in pattern"));
    }
    let repeats = match f.value("--repeats") {
        None => None,
        Some(v) => match v.trim().parse::<u16>() {
            Ok(n) if (1..=999).contains(&n) => Some(n),
            _ => return Err(usage(format!("`run`: --repeats is a whole number from 1 to 999, got `{v}`"))),
        },
    };
    let power_pct = match f.value("--power-pct") {
        None => None,
        Some(v) => {
            if !builtin {
                return Err(usage("`run`: --power-pct overrides built-in patterns only; set a project's power in its layers"));
            }
            let p = finite("run", "--power-pct", v)?;
            if p <= 0.0 || p > 100.0 {
                return Err(usage(format!("`run`: --power-pct must be above 0 and at most 100, got {p}")));
            }
            Some(p)
        }
    };
    Ok(Cmd::Run(RunOpts { job, layers, repeats, power_pct, allow_emission: f.has("--allow-emission"), device: f.device() }))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(s: &str) -> Result<Cmd, HwError> {
        let v: Vec<String> = s.split_whitespace().map(str::to_owned).collect();
        parse(&v)
    }

    fn is_usage(r: Result<Cmd, HwError>) -> bool {
        matches!(r, Err(HwError::Usage(_)))
    }

    #[test]
    fn parses_every_subcommand() {
        assert_eq!(p("detect").ok(), Some(Cmd::Detect));
        assert_eq!(p("bench start").ok(), Some(Cmd::Bench(BenchCmd::Start)));
        assert_eq!(p("bench end").ok(), Some(Cmd::Bench(BenchCmd::End)));
        let Ok(Cmd::Status(s)) = p("status --sim --machine jpt-m7-200-sim --trace t.jsonl") else { panic!("status") };
        assert!(s.device.sim);
        assert_eq!(s.device.machine, Some(MachineId("jpt-m7-200-sim".into())));
        assert_eq!(s.device.trace, Some(PathBuf::from("t.jsonl")));
        assert_eq!(p("ports").ok(), Some(Cmd::Ports(PortsOpts { watch: false, device: DeviceOpts::default() })));
        let Ok(Cmd::Ports(po)) = p("ports --watch --sim --machine jpt-m7-200-sim") else { panic!("ports") };
        assert!(po.watch && po.device.sim);
        assert!(is_usage(p("ports --watch --watch")));
        assert!(is_usage(p("ports 15")));
        assert!(is_usage(p("ports --write 1")));
        let Ok(Cmd::Profile(pr)) = p("profile set cal.negate.x true --machine jpt-m7-200") else { panic!("profile") };
        assert_eq!(pr.action, ProfileAction::Set { key: "cal.negate.x".into(), value: "true".into() });
        let Ok(Cmd::Frame(fr)) = p("frame --builtin corners --extent 110 --loop --style contour --source both --sim") else { panic!("frame") };
        assert_eq!(fr.job, JobSource::Builtin(Builtin::Corners));
        assert_eq!(fr.extent, Some(110.0));
        assert!(fr.looping);
        assert_eq!(fr.style, Some(FrameStyle::Contour));
        assert_eq!(fr.source, Some(FrameSource::JobAndToolLayers));
        let Ok(Cmd::Run(r)) = p("run card.olp --repeats 3 --allow-emission --sim") else { panic!("run") };
        assert_eq!(r.job, JobSource::Project(PathBuf::from("card.olp")));
        assert_eq!(r.repeats, Some(3));
        assert!(r.allow_emission);
        let Ok(Cmd::Run(r)) = p("run --builtin dot --power-pct 1 --allow-emission") else { panic!("run builtin") };
        assert_eq!(r.power_pct, Some(1.0));
        assert!(!r.device.sim);
    }

    #[test]
    fn negative_numbers_are_values_and_flags_need_values() {
        let Ok(Cmd::Profile(pr)) = p("profile set cal.offsetMm -0.5,-0.25") else { panic!("negative offset") };
        assert_eq!(pr.action, ProfileAction::Set { key: "cal.offsetMm".into(), value: "-0.5,-0.25".into() });
        let Ok(Cmd::Profile(pr)) = p("profile set workArea -50,-40,50,40 --machine jpt-m7-200") else { panic!("negative work area") };
        assert_eq!(pr.action, ProfileAction::Set { key: "workArea".into(), value: "-50,-40,50,40".into() });
        let Ok(Cmd::Profile(pr)) = p("profile set cal.offsetMm -.5") else { panic!("leading dot") };
        assert_eq!(pr.action, ProfileAction::Set { key: "cal.offsetMm".into(), value: "-.5".into() });
        // A valued flag never swallows the next flag; unknown single-dash words are still flags.
        for bad in [
            "frame a.olp --trace --sim",
            "run a.olp --machine --allow-emission",
            "status --machine --sim",
            "frame a.olp -x",
            "run --builtin dot --power-pct -1",
        ] {
            assert!(is_usage(p(bad)), "`{bad}` must be a usage error");
        }
    }

    #[test]
    fn cli_extent_refused_for_run() {
        assert!(is_usage(p("run --builtin corners --extent 110 --sim")));
        assert!(is_usage(p("run --builtin corners --sim --extent 100")));
        // And only corners may use it when framing.
        assert!(is_usage(p("frame --builtin box10 --extent 110 --sim")));
        assert!(is_usage(p("frame card.olp --extent 110")));
        assert!(is_usage(p("frame --builtin corners --extent -5")));
        assert!(is_usage(p("frame --builtin corners --extent NaN")));
        assert!(is_usage(p("frame --builtin corners --extent inf")));
    }

    #[test]
    fn usage_errors() {
        for bad in [
            "",
            "fire",
            "mark x.olp",
            "detect --sim",
            "status extra",
            "bench",
            "bench start now",
            "profile",
            "profile set onlykey",
            "frame",
            "frame a.olp b.olp",
            "frame a.olp --builtin dot",
            "frame --builtin triangle",
            "frame a.olp --style hull",
            "frame a.olp --source all",
            "frame a.olp --machine",
            "run a.olp --force",
            "run a.olp --repeats 0",
            "run a.olp --repeats 1000",
            "run a.olp --repeats x",
            "run a.olp --power-pct 5",
            "run --builtin dot --power-pct 0",
            "run --builtin dot --power-pct 100.5",
            "run --builtin dot --power-pct NaN",
            "run --builtin dot --layers l.json",
            "run a.olp --sim --sim",
        ] {
            assert!(is_usage(p(bad)), "`{bad}` must be a usage error");
        }
    }
}
