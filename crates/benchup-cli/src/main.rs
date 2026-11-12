//! `benchup`: read and edit BenchUp project files from a shell or an AI agent.
//!
//! Every command prints JSON on stdout. Exit status: 0 success, 1 the tool
//! reported an error (the file is unchanged), 2 usage or I/O error.
mod launch;
mod mcp;
mod project;

use benchup_tools::{definitions, is_mutating, mcp_tools};
use project::{Applied, Project};
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
  benchup tools [NAME] [--json]                    list tools, or show one tool's JSON schema
  benchup scene <file.bup>                         scene summary (objects, stock, joints)
  benchup get <file.bup> <id>                      one entity in detail, with its faces
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
  --brief          (call) leave out object summaries and bounds from results
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
    run_calls(project, calls, true)
}

fn run_calls(project: &mut Project, calls: &[(String, Value)], save: bool) -> Result<Output, String> {
    let batch = calls.len() > 1;
    Ok(match project.run(calls, save)? {
        Applied::Ok(mut results) if !batch => ok(results.pop().unwrap_or(Value::Null)),
        Applied::Ok(results) => ok(json!({"ok": true, "results": results})),
        Applied::Failed { result, .. } if !batch => Output { value: result, ok: false },
        Applied::Failed { index, result } => Output {
            value: json!({
                "ok": false,
                "failed_call": index + 1,
                "tool": calls[index].0,
                "result": result,
                "note": "nothing was changed; fix this call and run the whole batch again",
            }),
            ok: false,
        },
    })
}

/// Drops the bulky parts of tool results, keeping ids, errors and answers.
fn brief(value: &mut Value) {
    if let Some(map) = value.as_object_mut() {
        map.remove("objects");
        map.remove("bounds_mm");
        for key in ["results", "result"] {
            match map.get_mut(key) {
                Some(Value::Array(items)) => items.iter_mut().for_each(brief),
                Some(item @ Value::Object(_)) => brief(item),
                _ => {}
            }
        }
    }
}

fn tools(args: Args) -> Result<Output, String> {
    match args.positional.as_slice() {
        [] if args.flag("--json") => Ok(ok(Value::Array(mcp_tools()))),
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
        "tools" => tools(parse_args(argv, &["--json"], &[])?),
        "scene" => {
            let args = parse_args(argv, &["--allow-shared"], &[])?;
            args.expect(1)?;
            apply(&mut open(&args)?, &[("get_scene".into(), json!({}))])
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
            let mut output = run_calls(&mut open(&args)?, &calls, !args.flag("--dry-run"))?;
            if args.flag("--dry-run") {
                output.value["dry_run"] = json!(true);
            }
            if args.flag("--brief") {
                brief(&mut output.value);
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
            println!("{}", serde_json::to_string_pretty(&output.value).unwrap_or_default());
            if output.ok { ExitCode::SUCCESS } else { ExitCode::from(1) }
        }
        Err(message) => {
            eprintln!("benchup: {message}");
            ExitCode::from(2)
        }
    }
}
