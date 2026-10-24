use std::fs;
use std::path::Path;
use std::process;

use mmapstruct::traits::MmapRead;
use mmapstruct_schema::context::RuntimeSchemaCtx;
use mmapstruct_schema::{format_schema_value, Schema, SchemaValue};
use mmapview::runtime_json::{value_to_json, OutState, RenderOpts};
use mmapview::{query_from_generic, query_from_generic_limited, EvalResult, QueryLimits};

const VERSION: &str = env!("CARGO_PKG_VERSION");

const DEFAULT_LIMIT: usize = 200;
const DEFAULT_MAX_STRING: usize = 120;
const DEFAULT_MAX_BYTES: usize = 1024 * 1024;

fn print_usage() {
    eprintln!("Usage:");
    eprintln!("  mmapview q [options] <data-file> <query>      JSONL query (AI-friendly)");
    eprintln!("  mmapview describe [options] <data-file>       structure map (text)");
    eprintln!("  mmapview <data-file> <schema-file> <query>    DSL query mode (text)");
    eprintln!("  mmapview [options] <data-file>                Tree view mode");
    eprintln!("  mmapview --self-schema-path <output-path>     Extract self-schema");
    eprintln!();
    eprintln!("q options:");
    eprintln!("  --schema FILE   Explicit schema path (default: auto — embedded/sidecar/registry)");
    eprintln!("  --limit N       Max candidate results for filters/slices (default: {DEFAULT_LIMIT})");
    eprintln!("  --skip N        Skip first N candidates (default: 0)");
    eprintln!("  --count-only    Print only the number of matches (scans without limit)");
    eprintln!("  --text          Plain-text output instead of JSONL");
    eprintln!("  --max-string N  Truncate strings to N chars (default: {DEFAULT_MAX_STRING})");
    eprintln!("  --max-bytes N   Stop after N serialized bytes (default: {DEFAULT_MAX_BYTES}; 0 = off)");
    eprintln!("  --full          Expand containers fully (byte budget still applies)");
    eprintln!();
    eprintln!("describe options:");
    eprintln!("  --depth=N       Max nesting depth (default: 5)");
    eprintln!("  --schema FILE   Explicit schema path");
    eprintln!();
    eprintln!("Query examples:");
    eprintln!("  mmapview q data.mms \"/\"");
    eprintln!("  mmapview q data.mms \"/items[?price>100] | count()\"");
    eprintln!("  mmapview q data.mms --limit 50 \"/items[?name~sensor]\"");
    eprintln!("  mmapview q data.mms \"/items[999999]/x\"        # O(1) indexed access");
    eprintln!();
    eprintln!("Tree mode (auto-detected on non-TTY stdout):");
    eprintln!("  mmapview --tree data.mms   (--depth=N --max-array=N --max-string=N --no-values)");
}

struct Loaded {
    ctx: RuntimeSchemaCtx<'static>,
    root: SchemaValue,
}

fn leak(data: Vec<u8>) -> &'static [u8] {
    Box::leak(data.into_boxed_slice())
}

/// Load a data file + resolve schema (explicit → embedded → registry → sidecar).
fn load(file: &str, schema_opt: Option<&str>) -> Result<Loaded, String> {
    let data_vec = fs::read(file).map_err(|e| format!("Error reading data file '{file}': {e}"))?;
    let loaded = mmapview::schema::load_schema_full(&data_vec, schema_opt, Some(file))?;
    let payload_start = loaded.payload_range.start;
    let payload_end = loaded.payload_range.end;
    let data: &'static [u8] = leak(data_vec);
    let schema_bytes: &'static [u8] = leak(loaded.schema_bytes);
    let schema_reader = <Schema as MmapRead>::read(schema_bytes, 0..schema_bytes.len());
    let root_type = *schema_reader.root_type;
    let ctx = RuntimeSchemaCtx::new(&data[payload_start..payload_end], schema_bytes);
    let root = SchemaValue::new(0, (payload_end - payload_start) as u64, root_type);
    Ok(Loaded { ctx, root })
}

fn run_q(args: &[String]) {
    let mut schema_opt: Option<&str> = None;
    let mut limit = DEFAULT_LIMIT;
    let mut skip = 0usize;
    let mut count_only = false;
    let mut text_out = false;
    let mut max_string = DEFAULT_MAX_STRING;
    let mut max_bytes = DEFAULT_MAX_BYTES;
    let mut full = false;
    let mut positional: Vec<&str> = Vec::new();

    // Value of `--flag` / `--flag=value` starting at index i; returns
    // (value_str, next_index).
    fn flag_val<'a>(args: &'a [String], i: usize, name: &str) -> Option<(&'a str, usize)> {
        let eq = format!("{name}=");
        if args[i].starts_with(&eq) {
            return Some((&args[i][eq.len()..], i + 1));
        }
        if args[i] == name && i + 1 < args.len() {
            return Some((args[i + 1].as_str(), i + 2));
        }
        None
    }

    let mut i = 0;
    while i < args.len() {
        let a = &args[i];
        if let Some((v, ni)) = flag_val(args, i, "--schema") {
            schema_opt = Some(v);
            i = ni;
        } else if let Some((v, ni)) = flag_val(args, i, "--limit") {
            limit = v.parse().unwrap_or(DEFAULT_LIMIT);
            i = ni;
        } else if let Some((v, ni)) = flag_val(args, i, "--skip") {
            skip = v.parse().unwrap_or(0);
            i = ni;
        } else if let Some((v, ni)) = flag_val(args, i, "--max-string") {
            max_string = v.parse().unwrap_or(DEFAULT_MAX_STRING);
            i = ni;
        } else if let Some((v, ni)) = flag_val(args, i, "--max-bytes") {
            max_bytes = v.parse().unwrap_or(DEFAULT_MAX_BYTES);
            i = ni;
        } else if a == "--count-only" {
            count_only = true;
            i += 1;
        } else if a == "--text" {
            text_out = true;
            i += 1;
        } else if a == "--full" {
            full = true;
            i += 1;
        } else if !a.starts_with("--") {
            positional.push(a);
            i += 1;
        } else {
            eprintln!("Unknown q option: {a}");
            process::exit(1);
        }
    }

    let [file, query] = positional.as_slice() else {
        eprintln!("q mode requires exactly: <data-file> <query>");
        process::exit(1);
    };

    let loaded = match load(file, schema_opt) {
        Ok(l) => l,
        Err(e) => {
            eprintln!("{e}");
            process::exit(1);
        }
    };

    let limits = QueryLimits {
        skip,
        // count-only must see every match to report the true total.
        max: if count_only { None } else { Some(limit) },
    };

    let results = match query_from_generic_limited(query, &loaded.ctx, &loaded.root, &limits) {
        Ok(r) => r,
        Err(e) => {
            eprintln!("Query error: {e}");
            process::exit(1);
        }
    };

    if count_only {
        println!("{}", results.len());
        return;
    }

    if text_out {
        for (i, r) in results.iter().enumerate() {
            if i > 0 {
                println!();
            }
            print_eval_text(&loaded.ctx, r);
        }
        return;
    }

    // Default: JSONL
    let opts = RenderOpts {
        full,
        max_string,
        depth_cap: 8,
    };
    let mut state = OutState::default();
    use std::io::Write;
    let stdout = std::io::stdout();
    let mut out = stdout.lock();
    for r in &results {
        let j = match r {
            EvalResult::Buf(v) => value_to_json(&loaded.ctx, v, &opts),
            other => mmapview::runtime_json::scalar_to_json(other),
        };
        let line = serde_json::to_string(&j).unwrap_or_else(|_| "null".into());
        if !state.fits(line.len() + 1, max_bytes) {
            break;
        }
        let _ = writeln!(out, "{line}");
    }
    if state.truncated {
        let meta = serde_json::json!({
            "$meta": {
                "truncated": true,
                "bytes_out": state.bytes,
                "hint": "narrow the query, raise --max-bytes, or use --skip/--limit paging"
            }
        });
        let _ = writeln!(out, "{meta}");
        eprintln!(
            "mmapview q: output truncated at {} bytes (use --max-bytes=0 to disable)",
            state.bytes
        );
    }
}

fn print_eval_text(ctx: &RuntimeSchemaCtx<'_>, r: &EvalResult<SchemaValue>) {
    match r {
        EvalResult::Buf(v) => println!("{}", format_schema_value(ctx, v)),
        EvalResult::U64(n) => println!("{n}"),
        EvalResult::I64(n) => println!("{n}"),
        EvalResult::F64(n) => {
            if *n == n.trunc() && !n.is_infinite() {
                println!("{}", n.trunc() as i64);
            } else {
                println!("{n}");
            }
        }
    }
}

fn run_describe(args: &[String]) {
    let mut schema_opt: Option<&str> = None;
    let mut depth = 5usize;
    let mut positional: Vec<&str> = Vec::new();
    let mut i = 0;
    while i < args.len() {
        let a = &args[i];
        if a == "--schema" && i + 1 < args.len() {
            schema_opt = Some(&args[i + 1]);
            i += 2;
        } else if let Some(v) = a.strip_prefix("--depth=") {
            depth = v.parse().unwrap_or(5);
            i += 1;
        } else if !a.starts_with("--") {
            positional.push(a);
            i += 1;
        } else {
            eprintln!("Unknown describe option: {a}");
            process::exit(1);
        }
    }
    let [file] = positional.as_slice() else {
        eprintln!("describe mode requires exactly: <data-file>");
        process::exit(1);
    };
    let loaded = match load(file, schema_opt) {
        Ok(l) => l,
        Err(e) => {
            eprintln!("{e}");
            process::exit(1);
        }
    };
    let opts = mmapview::describe::DescribeOpts { depth };
    print!("{}", mmapview::describe::describe(&loaded.ctx, &loaded.root, &opts));
}

fn main() {
    let args: Vec<String> = std::env::args().collect();

    if args.iter().any(|a| a == "--version" || a == "-V") {
        println!("mmapview v{}", VERSION);
        return;
    }

    if args.iter().any(|a| a == "--help" || a == "-h") {
        print_usage();
        return;
    }

    if args.len() < 2 {
        print_usage();
        process::exit(1);
    }

    // Subcommands
    match args[1].as_str() {
        "q" | "query" => return run_q(&args[2..]),
        "describe" => return run_describe(&args[2..]),
        _ => {}
    }

    // Handle --self-schema-path subcommand
    if args.len() >= 3 && args[1] == "--self-schema-path" {
        let path = &args[2];
        match fs::write(path, mmapstruct_schemagen::SELF_SCHEMA_MMS) {
            Ok(_) => {
                println!(
                    "Wrote self-schema ({} bytes) to {}",
                    mmapstruct_schemagen::SELF_SCHEMA_MMS.len(),
                    path
                );
                return;
            }
            Err(e) => {
                eprintln!("Error writing self-schema to '{}': {}", path, e);
                process::exit(1);
            }
        }
    }

    // Determine mode: tree vs legacy query
    let explicit_tree = args.iter().any(|a| a == "--tree");
    let auto_tree = !std::io::IsTerminal::is_terminal(&std::io::stdout()) && args.len() <= 3;

    if explicit_tree || auto_tree {
        let data_path = args
            .iter()
            .rev()
            .find(|a| !a.starts_with("--"))
            .map(|s| s.as_str());

        let data_path = match data_path {
            Some(p) if Path::new(p).exists() => p.to_string(),
            _ => {
                eprintln!("Error: data file not found in arguments.");
                eprintln!("Usage: mmapview [options] <data-file>");
                process::exit(1);
            }
        };

        let data = match fs::read(&data_path) {
            Ok(d) => d,
            Err(e) => {
                eprintln!("Error reading file '{}': {}", data_path, e);
                process::exit(1);
            }
        };

        let mut opts = mmapview::tree::TreeOptions::default();
        for arg in &args {
            if let Some(v) = arg.strip_prefix("--depth=") {
                opts.depth = v.parse().unwrap_or(4);
            } else if let Some(v) = arg.strip_prefix("--max-array=") {
                opts.max_array = v.parse().unwrap_or(20);
            } else if let Some(v) = arg.strip_prefix("--max-string=") {
                opts.max_string = v.parse().unwrap_or(80);
            } else if arg == "--no-values" {
                opts.no_values = true;
            }
        }

        match mmapview::tree::format_file(&data, &opts) {
            Ok(tree) => print!("{}", tree),
            Err(e) => {
                eprintln!("Tree error: {}", e);
                process::exit(1);
            }
        }
        return;
    }

    // Legacy query mode: exactly 3 positional args
    if args.len() < 4 {
        print_usage();
        process::exit(1);
    }

    let data_path = &args[1];
    let schema_path = &args[2];
    let query = &args[3];

    let data = match fs::read(data_path) {
        Ok(d) => d,
        Err(e) => {
            eprintln!("Error reading data file '{}': {}", data_path, e);
            process::exit(1);
        }
    };

    let schema_bytes = match fs::read(schema_path) {
        Ok(d) => d,
        Err(e) => {
            eprintln!("Error reading schema file '{}': {}", schema_path, e);
            process::exit(1);
        }
    };

    // Strip any mmapstruct header — schema file may be raw or v2
    let (schema_payload, _embedded, _hash, _crc) =
        mmapstruct::file_reader::parse_header(&schema_bytes);
    let schema_bytes = schema_payload.to_vec();

    let schema_reader = <Schema as MmapRead>::read(&schema_bytes, 0..schema_bytes.len());
    let root_type = *schema_reader.root_type;

    let ctx = RuntimeSchemaCtx::new(&data, &schema_bytes);
    let root = SchemaValue::new(0, data.len() as u64, root_type);

    match query_from_generic(query, &ctx, &root) {
        Ok(results) => {
            for (i, r) in results.iter().enumerate() {
                if i > 0 {
                    println!();
                }
                print_eval_text(&ctx, r);
            }
        }
        Err(e) => {
            eprintln!("Query error: {}", e);
            process::exit(1);
        }
    }
}
