//! Command-line front end for the Tessera language.
//!
//! Subcommands:
//!
//! ```text
//! tessera run   <file.tsr>            compile and execute source
//! tessera build <file.tsr> [-o out]   compile source to a .tmod module
//! tessera exec  <file.tmod>           load and execute a compiled module
//! tessera dis   <file.tsr|.tmod>      disassemble source or a module
//! tessera inspect <file.tsr|.tmod>    print a structured module report
//! tessera manifest <Tessera.toml>     validate and summarize a project
//! tessera doc <builtin>               show standard-library help
//! ```
//!
//! The tool is deliberately non-interactive: it takes file paths on the command
//! line, never prompts, and exits with a non-zero status on error.

use std::process::ExitCode;

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().collect();
    if args.len() < 3 {
        eprintln!("{}", USAGE);
        return ExitCode::from(2);
    }
    let cmd = args[1].as_str();
    let path = args[2].as_str();

    let result = match cmd {
        "run" => cmd_run(path),
        "build" => cmd_build(path, out_flag(&args)),
        "exec" => cmd_exec(path),
        "dis" => cmd_dis(path),
        "stat" => cmd_stat(path),
        "fmt" => cmd_fmt(path),
        "json" => cmd_json(path),
        "hexdump" => cmd_hexdump(path),
        "inspect" => cmd_inspect(path),
        "manifest" => cmd_manifest(path),
        "doc" => cmd_doc(path),
        other => {
            eprintln!("unknown subcommand `{other}`\n{USAGE}");
            return ExitCode::from(2);
        }
    };

    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(msg) => {
            eprintln!("error: {msg}");
            ExitCode::FAILURE
        }
    }
}

const USAGE: &str =
    "usage: tessera <run|build|exec|dis|stat|fmt|json|hexdump|inspect|manifest|doc> <file-or-name> [-o output]";

fn out_flag(args: &[String]) -> Option<String> {
    let mut i = 3;
    while i < args.len() {
        if args[i] == "-o" && i + 1 < args.len() {
            return Some(args[i + 1].clone());
        }
        i += 1;
    }
    None
}

fn cmd_run(path: &str) -> Result<(), String> {
    let src = read(path)?;
    let (value, output) = tessera::run_source(&src).map_err(|e| e.to_string())?;
    print!("{output}");
    println!("=> {}", value);
    Ok(())
}

fn cmd_build(path: &str, out: Option<String>) -> Result<(), String> {
    let src = read(path)?;
    let bytes = tessera::compile_to_bytes(&src, stem(path)).map_err(|e| e.to_string())?;
    let out_path = out.unwrap_or_else(|| format!("{}.tmod", stem(path)));
    std::fs::write(&out_path, &bytes).map_err(|e| format!("writing {out_path}: {e}"))?;
    println!("wrote {} ({} bytes)", out_path, bytes.len());
    Ok(())
}

fn cmd_exec(path: &str) -> Result<(), String> {
    let bytes = std::fs::read(path).map_err(|e| format!("reading {path}: {e}"))?;
    let value = tessera::run_module_bytes(&bytes).map_err(|e| e.to_string())?;
    println!("=> {}", value);
    Ok(())
}

fn cmd_dis(path: &str) -> Result<(), String> {
    let module = if path.ends_with(".tmod") {
        let bytes = std::fs::read(path).map_err(|e| format!("reading {path}: {e}"))?;
        tessera::load_module(&bytes).map_err(|e| e.to_string())?
    } else {
        let src = read(path)?;
        tessera::compile_source(&src, stem(path)).map_err(|e| e.to_string())?
    };
    print!("{}", tessera::disasm::disassemble(&module));
    Ok(())
}

fn cmd_stat(path: &str) -> Result<(), String> {
    let module = load_any(path)?;
    let stats = tessera::analysis::module_stats(&module);
    print!("{}", stats.report());
    Ok(())
}

fn cmd_fmt(path: &str) -> Result<(), String> {
    let src = read(path)?;
    let tokens = tessera::lexer::tokenize(&src).map_err(|e| e.to_string())?;
    let program = tessera::parser::parse(tokens).map_err(|e| e.to_string())?;
    print!("{}", tessera::fmt::pretty_program(&program));
    Ok(())
}

fn cmd_json(path: &str) -> Result<(), String> {
    let module = load_any(path)?;
    println!("{}", tessera::fmt::module_to_json(&module));
    Ok(())
}

fn cmd_hexdump(path: &str) -> Result<(), String> {
    let bytes = std::fs::read(path).map_err(|e| format!("reading {path}: {e}"))?;
    print!("{}", tessera::text::hexdump(&bytes));
    Ok(())
}

fn cmd_inspect(path: &str) -> Result<(), String> {
    let module = load_any(path)?;
    let report = tessera::inspect::inspect_module(&module);
    print!("{}", report.render_text());
    Ok(())
}

fn cmd_manifest(path: &str) -> Result<(), String> {
    let src = read(path)?;
    let manifest = tessera::manifest::parse_manifest(&src).map_err(|e| e.to_string())?;
    print!("{}", tessera::manifest::manifest_summary(&manifest));
    Ok(())
}

fn cmd_doc(name: &str) -> Result<(), String> {
    match tessera::stdlib::render_builtin(name) {
        Some(text) => {
            println!("{text}");
            Ok(())
        }
        None => Err(format!("unknown builtin `{name}`")),
    }
}

/// Load a module from either compiled `.tmod` bytes or `.tsr` source.
fn load_any(path: &str) -> Result<tessera::Module, String> {
    if path.ends_with(".tmod") {
        let bytes = std::fs::read(path).map_err(|e| format!("reading {path}: {e}"))?;
        tessera::load_module(&bytes).map_err(|e| e.to_string())
    } else {
        let src = read(path)?;
        tessera::compile_source(&src, stem(path)).map_err(|e| e.to_string())
    }
}

fn read(path: &str) -> Result<String, String> {
    std::fs::read_to_string(path).map_err(|e| format!("reading {path}: {e}"))
}

fn stem(path: &str) -> &str {
    let name = path.rsplit(['/', '\\']).next().unwrap_or(path);
    name.split('.').next().unwrap_or(name)
}
