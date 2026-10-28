mod api;
mod new_module;

use std::path::PathBuf;
use std::process::ExitCode;

const USAGE: &str = "usage: cargo xtask new-module <name> [--dir <parent directory>] | cargo xtask api [--check]";

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match run(&args) {
        Ok(message) => {
            println!("{message}");
            ExitCode::SUCCESS
        }
        Err(message) => {
            eprintln!("{message}");
            ExitCode::FAILURE
        }
    }
}

fn run(args: &[String]) -> Result<String, String> {
    match args.first().map(String::as_str) {
        Some("new-module") => {
            let (name, dir) = parse_new_module(&args[1..])?;
            let created = new_module::scaffold(&name, &dir)?;
            Ok(new_module::report(&name, &created))
        }
        Some("api") => {
            let check = match args.get(1).map(String::as_str) {
                None => false,
                Some("--check") => true,
                Some(_) => return Err(USAGE.to_string()),
            };
            api::run(&std::env::current_dir().map_err(|e| e.to_string())?, check)
        }
        _ => Err(USAGE.to_string()),
    }
}

fn parse_new_module(args: &[String]) -> Result<(String, PathBuf), String> {
    let mut name = None;
    let mut dir = PathBuf::from("src");
    let mut iter = args.iter();
    while let Some(arg) = iter.next() {
        match arg.as_str() {
            "--dir" => dir = PathBuf::from(iter.next().ok_or(USAGE)?),
            other if name.is_none() => name = Some(other.to_string()),
            _ => return Err(USAGE.to_string()),
        }
    }
    Ok((name.ok_or(USAGE)?, dir))
}
