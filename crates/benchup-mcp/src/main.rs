use benchup_mcp::{file_mode::FileBackend, live, serve};
use std::io::{stdin, stdout};
use std::process::ExitCode;

const USAGE: &str = "usage:\n  benchup-mcp file <project.bup> [--allow-shared]\n  \
benchup-mcp live [--port 47615] [--token T] [--allow-origin ORIGIN]...\n\
(live reads the token from BENCHUP_AGENT_TOKEN when --token is absent)";

fn run() -> Result<(), String> {
    let mut args = std::env::args().skip(1);
    let mode = args.next().ok_or(USAGE)?;
    let rest: Vec<String> = args.collect();
    match mode.as_str() {
        "file" => {
            let mut path = None;
            let mut allow_shared = false;
            for arg in &rest {
                match arg.as_str() {
                    "--allow-shared" => allow_shared = true,
                    a if a.starts_with("--") => return Err(format!("unknown option {a}\n{USAGE}")),
                    a if path.is_none() => path = Some(a.to_owned()),
                    _ => return Err(USAGE.into()),
                }
            }
            let mut backend = FileBackend::open(path.ok_or(USAGE)?, allow_shared)?;
            eprintln!("benchup-mcp: editing {}", backend.document().name);
            serve(stdin().lock(), stdout().lock(), &mut backend).map_err(|e| e.to_string())
        }
        "live" => {
            let mut port = live::DEFAULT_PORT;
            let mut token = std::env::var("BENCHUP_AGENT_TOKEN").ok();
            let mut origins = Vec::new();
            let mut it = rest.into_iter();
            while let Some(arg) = it.next() {
                let mut value = |name: &str| it.next().ok_or(format!("{name} needs a value"));
                match arg.as_str() {
                    "--port" => port = value("--port")?.parse().map_err(|_| "invalid --port")?,
                    "--token" => token = Some(value("--token")?),
                    "--allow-origin" => origins.push(value("--allow-origin")?),
                    _ => return Err(format!("unknown option {arg}\n{USAGE}")),
                }
            }
            let token = token.ok_or("a pairing token is required: pass --token or set BENCHUP_AGENT_TOKEN")?;
            let mut config = live::LiveConfig::new(token);
            config.port = port;
            config.extra_origins = origins;
            let mut bridge = live::Bridge::start(config)?;
            eprintln!("benchup-mcp: waiting for BenchUp on ws://127.0.0.1:{}", bridge.port());
            serve(stdin().lock(), stdout().lock(), &mut bridge).map_err(|e| e.to_string())
        }
        _ => Err(USAGE.into()),
    }
}

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(message) => {
            eprintln!("benchup-mcp: {message}");
            ExitCode::FAILURE
        }
    }
}
