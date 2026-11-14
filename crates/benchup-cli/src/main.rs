//! `benchup`: read and edit BenchUp project files from a shell or an AI agent.
//!
//! Every command prints JSON on stdout. Exit status: 0 success, 1 the tool
//! reported an error (the file is unchanged), 2 usage or I/O error.
mod launch;
mod mcp;
mod project;

use benchup_tools::{definitions, is_mutating, mcp_tools};
use project::Project;
use serde_json::{Value, json};
use std::io::{Read, stdin, stdout};
use std::path::Path;
use std::process::ExitCode;

const USAGE: &str = "\
benchup: read and edit BenchUp (.bup) projects. Lengths are millimetres, Z is up.
New here? Run `benchup guide` first: conventions, workflow and woodworking recipes.

usage:
  benchup new <file.bup> [--name NAME] [--force]   create an empty project
  benchup guide                                    how to model furniture with these tools
  benchup tools [NAME] [--json] [--names]          list tools (--names: names only), or show one
                                                   tool's JSON schema
  benchup scene <file.bup> [FILTER]                scene summary (objects, stock, joints); FILTER
                                                   is get_scene's JSON, e.g. '{\"depth\":0}'
  benchup get <file.bup> <id>                      one entity in detail, with its faces
  benchup render <file.bup> [-o OUT.png] [--view VIEW] [--size WxH]
                                                   a PNG of the model (iso, front, back, left,
                                                   right, top, bottom; default iso, 1000x750;
                                                   64 to 2400 pixels a side)
  benchup call <file.bup> <tool> [ARGS|-]          run a tool; ARGS is a JSON object (- reads stdin)
  benchup call <file.bup> --batch <FILE|->         run JSON lines {\"tool\": .., \"arguments\": {..}}
                                                   as one all-or-nothing undo step; \"$N.created[0]\"
                                                   in arguments is a value from call N (from 1)
  benchup undo <file.bup>                          undo the last change
  benchup redo <file.bup>                          redo the last undone change
  benchup mcp <file.bup>                           serve the tools over MCP (stdio)
  benchup open <file.bup> [--app EXECUTABLE]       open the project in the BenchUp desktop app
                                                   (or in EXECUTABLE, default $BENCHUP_APP)

options:
  --dry-run        (call) report the results without changing the file
  --brief          (call) leave out the object summaries of changing calls
  --allow-shared   edit the local copy of a shared project

Every change is one undo step in BenchUp and the file is saved atomically.
Each command starts at the top level of the model: a group opened with
enter_object stays open only until the end of that command, so put enter_object
and the calls that need it in one --batch.
Output is JSON. Exit status: 0 ok, 1 tool error (file unchanged), 2 usage or I/O error.";

/// What a command produced: JSON to print and whether it succeeded.
struct Output {
    value: Value,
    ok: bool,
}

fn ok(value: Value) -> Output {
    Output { value, ok: true }
}

/// Splits `args` into positionals and the flags this command accepts.
struct Args {
    positional: Vec<String>,
    flags: Vec<String>,
    values: Vec<(String, String)>,
}

fn parse_args(args: Vec<String>, flags: &[&str], valued: &[&str]) -> Result<Args, String> {
    let mut out = Args { positional: Vec::new(), flags: Vec::new(), values: Vec::new() };
    let mut it = args.into_iter();
    while let Some(arg) = it.next() {
        if flags.contains(&arg.as_str()) {
            out.flags.push(arg);
        } else if valued.contains(&arg.as_str()) {
            let value = it.next().ok_or(format!("{arg} needs a value"))?;
            out.values.push((arg, value));
        } else if arg.starts_with("--") {
            return Err(format!("unknown option {arg}"));
        } else {
            out.positional.push(arg);
        }
    }
    Ok(out)
}

impl Args {
    fn flag(&self, name: &str) -> bool {
        self.flags.iter().any(|f| f == name)
    }
    fn value(&self, name: &str) -> Option<&str> {
        self.values.iter().find(|(n, _)| n == name).map(|(_, v)| v.as_str())
    }
    /// Exactly `n` positionals, or a usage error.
    fn expect(&self, n: usize) -> Result<&[String], String> {
        if self.positional.len() == n { Ok(&self.positional) } else { Err(USAGE.into()) }
    }
}

fn read_input(source: &str) -> Result<String, String> {
    if source == "-" {
        let mut text = String::new();
        stdin().read_to_string(&mut text).map_err(|e| format!("cannot read stdin: {e}"))?;
        Ok(text)
    } else {
        std::fs::read_to_string(source).map_err(|e| format!("cannot read {source}: {e}"))
    }
}

fn parse_object(text: &str, what: &str) -> Result<Value, String> {
    if text.trim().is_empty() {
        return Ok(json!({}));
    }
    match serde_json::from_str::<Value>(text) {
        Ok(value @ Value::Object(_)) => Ok(value),
        Ok(_) => Err(format!("{what} must be a JSON object")),
        Err(e) => Err(format!("{what} is not valid JSON: {e}")),
    }
}

/// One batch line: `{"tool": "move", "arguments": {...}}` (`name` is accepted for `tool`).
fn parse_batch(text: &str) -> Result<Vec<(String, Value)>, String> {
    let mut calls = Vec::new();
    for (number, line) in text.lines().enumerate() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let entry = parse_object(line, &format!("batch line {}", number + 1))?;
        let name = entry
            .get("tool")
            .or_else(|| entry.get("name"))
            .and_then(Value::as_str)
            .ok_or(format!("batch line {}: missing \"tool\"", number + 1))?;
        let arguments = entry.get("arguments").cloned().unwrap_or_else(|| json!({}));
        if !arguments.is_object() {
            return Err(format!("batch line {}: \"arguments\" must be an object", number + 1));
        }
        calls.push((name.to_owned(), arguments));
    }
    if calls.is_empty() {
        return Err("the batch has no calls".into());
    }
    Ok(calls)
}

fn apply(project: &mut Project, calls: &[(String, Value)]) -> Result<Output, String> {
    run_calls(project, calls, true, false)
}

fn run_calls(project: &mut Project, calls: &[(String, Value)], save: bool, batch: bool) -> Result<Output, String> {
    let (value, ok) = project::report(calls, project.run(calls, save)?, batch);
    Ok(Output { value, ok })
}

/// Pretty JSON with the answer first: `ok`, then the error and hint of a
/// failure, then the rest in name order.
fn pretty(value: &Value) -> String {
    const FIRST: [&str; 6] = ["ok", "error", "hint", "failed_call", "tool", "dry_run"];
    let Value::Object(map) = value else { return serde_json::to_string_pretty(value).unwrap_or_default() };
    let keys = FIRST.iter().copied().filter(|k| map.contains_key(*k)).chain(map.keys().map(String::as_str).filter(|k| !FIRST.contains(k)));
    let fields: Vec<String> = keys
        .map(|key| format!("  {}: {}", json!(key), serde_json::to_string_pretty(&map[key]).unwrap_or_default().replace('\n', "\n  ")))
        .collect();
    if fields.is_empty() { "{}".into() } else { format!("{{\n{}\n}}", fields.join(",\n")) }
}

fn tools(args: Args) -> Result<Output, String> {
    match args.positional.as_slice() {
        [] if args.flag("--json") => Ok(ok(Value::Array(mcp_tools()))),
        [] if args.flag("--names") => Ok(ok(json!(definitions().into_iter().map(|d| d.name).collect::<Vec<_>>()))),
        [] => {
            let list: Vec<Value> = definitions()
                .into_iter()
                .map(|d| json!({"name": d.name, "changes_model": is_mutating(&d.name), "description": d.description}))
                .collect();
            Ok(ok(Value::Array(list)))
        }
        [name] => definitions()
            .into_iter()
            .find(|d| &d.name == name)
            .map(|d| ok(json!({"name": d.name, "description": d.description, "inputSchema": d.input_schema})))
            .ok_or(format!("unknown tool {name}; run `benchup tools` for the list")),
        _ => Err(USAGE.into()),
    }
}

fn run(mut argv: Vec<String>) -> Result<Output, String> {
    if argv.is_empty() || matches!(argv[0].as_str(), "-h" | "--help" | "help") {
        println!("{USAGE}");
        std::process::exit(if argv.is_empty() { 2 } else { 0 });
    }
    let command = argv.remove(0);
    let open = |args: &Args| Project::open(&args.positional[0], args.flag("--allow-shared"));
    match command.as_str() {
        "new" => {
            let args = parse_args(argv, &["--force"], &["--name"])?;
            let path = Path::new(&args.expect(1)?[0]);
            let doc = Project::create(path, args.value("--name"), args.flag("--force"))?;
            Ok(ok(json!({"ok": true, "created": path.display().to_string(), "name": doc.name})))
        }
        "tools" => tools(parse_args(argv, &["--json", "--names"], &[])?),
        "scene" => {
            let args = parse_args(argv, &["--allow-shared"], &[])?;
            let filter = match args.positional.len() {
                1 => json!({}),
                2 => parse_object(&args.positional[1], "FILTER")?,
                _ => return Err(USAGE.into()),
            };
            apply(&mut open(&args)?, &[("get_scene".into(), filter)])
        }
        "render" => {
            let args = parse_args(argv, &["--allow-shared"], &["-o", "--output", "--view", "--size"])?;
            let path = &args.expect(1)?[0];
            let view = args.value("--view").unwrap_or("iso");
            let (width, height) = match args.value("--size") {
                None => (1000, 750),
                Some(size) => size
                    .split_once('x')
                    .and_then(|(w, h)| Some((w.trim().parse().ok()?, h.trim().parse().ok()?)))
                    .ok_or("--size is WIDTHxHEIGHT in pixels, e.g. 1200x900")?,
            };
            let project = open(&args)?;
            let rendered = benchup_tools::render::render(&project.doc.model, view, width, height)?;
            let output = match args.value("-o").or(args.value("--output")) {
                Some(output) => std::path::PathBuf::from(output),
                None => Path::new(path).with_extension(format!("{view}.png")),
            };
            std::fs::write(&output, &rendered.png).map_err(|e| format!("cannot write {}: {e}", output.display()))?;
            let mut value = rendered.info;
            value["ok"] = json!(true);
            value["file"] = json!(output.display().to_string());
            Ok(ok(value))
        }
        "get" => {
            let args = parse_args(argv, &["--allow-shared"], &[])?;
            let id: u64 = args.expect(2)?[1].parse().map_err(|_| "the id must be a number".to_owned())?;
            apply(&mut open(&args)?, &[("get_object".into(), json!({"id": id}))])
        }
        "guide" => {
            println!("{}", benchup_tools::GUIDE);
            std::process::exit(0);
        }
        "call" => {
            let args = parse_args(argv, &["--allow-shared", "--dry-run", "--brief"], &["--batch"])?;
            let calls = match (args.value("--batch"), args.positional.as_slice()) {
                (Some(source), [_]) => parse_batch(&read_input(source)?)?,
                (None, [_, tool]) => vec![(tool.clone(), json!({}))],
                (None, [_, tool, arguments]) => {
                    let text = if arguments == "-" { read_input("-")? } else { arguments.clone() };
                    vec![(tool.clone(), parse_object(&text, "ARGS")?)]
                }
                _ => return Err(USAGE.into()),
            };
            let batch = args.value("--batch").is_some();
            let mut output = run_calls(&mut open(&args)?, &calls, !args.flag("--dry-run"), batch)?;
            if args.flag("--dry-run") {
                output.value["dry_run"] = json!(true);
            }
            if args.flag("--brief") {
                project::brief(&mut output.value);
            }
            Ok(output)
        }
        "undo" | "redo" => {
            let args = parse_args(argv, &["--allow-shared"], &[])?;
            args.expect(1)?;
            let value = open(&args)?.step_history(command == "redo")?;
            let ok = value["ok"] == true;
            Ok(Output { value, ok })
        }
        "open" => {
            let args = parse_args(argv, &[], &["--app"])?;
            let path = &args.expect(1)?[0];
            let app = args.value("--app").map(str::to_owned).or_else(|| {
                std::env::var("BENCHUP_APP").ok().filter(|app| !app.is_empty())
            });
            Ok(ok(launch::open(path, app.as_deref())?))
        }
        "mcp" => {
            let args = parse_args(argv, &["--allow-shared"], &[])?;
            args.expect(1)?;
            let mut project = open(&args)?;
            eprintln!("benchup: serving {} over MCP on stdio", project.doc.name);
            mcp::serve(stdin().lock(), stdout().lock(), &mut project).map_err(|e| e.to_string())?;
            std::process::exit(0);
        }
        _ => Err(format!("unknown command {command}\n\n{USAGE}")),
    }
}

fn main() -> ExitCode {
    match run(std::env::args().skip(1).collect()) {
        Ok(output) => {
            println!("{}", pretty(&output.value));
            if output.ok { ExitCode::SUCCESS } else { ExitCode::from(1) }
        }
        Err(message) => {
            eprintln!("benchup: {message}");
            ExitCode::from(2)
        }
    }
}
