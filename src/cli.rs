//! Running a macro from the command line, without opening the editor:
//!
//! ```text
//! macroflow run my-macro.json [--trigger farm] [--repeat 3] [--var gold=100] [--no-wait]
//! ```

use crate::engine::{self, EndKind, LogLevel, WorkerMsg};
use crate::model::{command_name, load_macro_file};
use std::path::PathBuf;
use std::sync::atomic::AtomicBool;
use std::sync::Arc;

const USAGE: &str = "\
Usage: macroflow run <macro.json> [options]

Plays a macro without opening the editor, prints its log, and exits when it ends.
Press Esc to stop it, as usual.

Options:
  --trigger <name>   start at this Trigger step instead of Start
  --repeat <n>       run the whole macro n times (0 = until stopped)
  --var <name=value> set a variable before the first step (can be repeated)
  --no-wait          skip the macro's start delay
  --quiet            only print why it ended

Exit codes: 0 finished, 1 something went wrong (or a path with nothing
connected), 2 bad arguments or file, 130 stopped with Esc.";

struct Options {
    file: PathBuf,
    trigger: Option<String>,
    repeat: Option<u32>,
    vars: Vec<(String, String)>,
    no_wait: bool,
    quiet: bool,
}

fn parse(args: &[String]) -> Result<Options, String> {
    let mut it = args.iter();
    let mut file = None;
    let mut o = Options { file: PathBuf::new(), trigger: None, repeat: None, vars: Vec::new(), no_wait: false, quiet: false };
    while let Some(a) = it.next() {
        let mut value = |name: &str| it.next().cloned().ok_or_else(|| format!("{name} needs a value."));
        match a.as_str() {
            "--trigger" => o.trigger = Some(value("--trigger")?),
            "--repeat" => {
                let v = value("--repeat")?;
                o.repeat = Some(v.parse().map_err(|_| format!("--repeat needs a whole number, not “{v}”."))?);
            }
            "--var" => {
                let v = value("--var")?;
                let (k, val) = v.split_once('=').ok_or_else(|| format!("--var needs name=value, not “{v}”."))?;
                if !crate::model::is_ident(k.trim()) {
                    return Err(format!("“{}” can't be a variable name.", k.trim()));
                }
                o.vars.push((k.trim().to_string(), val.to_string()));
            }
            "--no-wait" => o.no_wait = true,
            "--quiet" | "-q" => o.quiet = true,
            s if s.starts_with('-') => return Err(format!("Unknown option {s}.")),
            s if file.is_none() => file = Some(PathBuf::from(s)),
            s => return Err(format!("Only one macro file can be run at a time (got “{s}” too).")),
        }
    }
    o.file = file.ok_or("Which macro file? e.g. macroflow run my-macro.json")?;
    Ok(o)
}

/// If the command line asks for something other than the editor, do it and return
/// the exit code. None = open the editor as usual.
pub fn main(args: &[String]) -> Option<i32> {
    match args.first().map(|s| s.as_str()) {
        Some("run") => Some(run(&args[1..])),
        Some("--help") | Some("-h") | Some("help") => {
            println!("{USAGE}");
            Some(0)
        }
        _ => None,
    }
}

fn run(args: &[String]) -> i32 {
    let o = match parse(args) {
        Ok(o) => o,
        Err(e) => {
            eprintln!("{e}\n\n{USAGE}");
            return 2;
        }
    };
    let (mut doc, warnings) = match load_macro_file(&o.file) {
        Ok(d) => d,
        Err(e) => {
            eprintln!("Couldn't open {}: {e}", o.file.display());
            return 2;
        }
    };
    for w in warnings {
        eprintln!("Note: {w}");
    }
    let from = match &o.trigger {
        Some(t) => match doc.trigger_named(t) {
            Some(id) => Some(id),
            None => {
                let names: Vec<String> = doc.triggers().into_iter().map(|(_, n)| format!("/{}", command_name(&n))).collect();
                eprintln!(
                    "{} has no trigger called /{}.{}",
                    o.file.display(),
                    command_name(t),
                    if names.is_empty() { String::new() } else { format!(" It has: {}", names.join(", ")) }
                );
                return 2;
            }
        },
        None => None,
    };
    if let Some(r) = o.repeat {
        doc.repeat = r;
        if let Some(id) = from {
            if let Some(n) = doc.node_mut(id) {
                if let crate::model::NodeKind::Trigger { repeat, .. } = &mut n.kind {
                    *repeat = r;
                }
            }
        }
    }
    if o.no_wait {
        doc.start_delay_ms = 0;
    }
    if !o.quiet {
        println!("▶ Running {} (Esc stops it)", doc.name);
    }
    let (tx, rx) = std::sync::mpsc::channel();
    let stop = Arc::new(AtomicBool::new(false));
    engine::run_macro(
        doc,
        tx,
        stop,
        Arc::new(AtomicBool::new(false)),
        Arc::new(AtomicBool::new(false)),
        from,
        o.vars,
    );
    while let Ok(msg) = rx.recv() {
        match msg {
            WorkerMsg::Log(e) if !o.quiet => {
                let mark = match e.level {
                    LogLevel::Step => "•",
                    LogLevel::Good => "✓",
                    LogLevel::Warn => "!",
                    LogLevel::Bad => "✗",
                    LogLevel::Info => " ",
                };
                println!("{:>7.2}s {mark} {}", e.ms as f64 / 1000.0, e.text);
            }
            WorkerMsg::Failed(e) => {
                eprintln!("{e}");
                return 1;
            }
            WorkerMsg::RunFinished(end) => {
                if o.quiet {
                    println!("{}", end.message);
                }
                return match end.kind {
                    EndKind::Stopped => 130,
                    EndKind::Error => 1,
                    EndKind::Finished if end.unexpected() => 1,
                    EndKind::Finished => 0,
                };
            }
            _ => {}
        }
    }
    1
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(s: &str) -> Vec<String> {
        s.split_whitespace().map(String::from).collect()
    }

    #[test]
    fn reads_options() {
        let o = parse(&args("m.json --trigger farm --repeat 3 --var gold=10 --var name=a=b --no-wait")).unwrap();
        assert_eq!(o.file, PathBuf::from("m.json"));
        assert_eq!(o.trigger.as_deref(), Some("farm"));
        assert_eq!(o.repeat, Some(3));
        assert_eq!(o.vars, vec![("gold".into(), "10".into()), ("name".into(), "a=b".into())]);
        assert!(o.no_wait);
        assert!(parse(&args("")).is_err());
        assert!(parse(&args("m.json --repeat x")).is_err());
        assert!(parse(&args("m.json --var 1x=2")).is_err());
        assert!(parse(&args("m.json --bogus")).is_err());
        assert!(main(&args("edit")).is_none());
    }
}
