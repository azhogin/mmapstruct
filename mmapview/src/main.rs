use std::fs;
use std::path::Path;
use std::process;

use mmapstruct::traits::MmapRead;
use mmapstruct_schema::context::RuntimeSchemaCtx;
use mmapstruct_schema::{format_schema_value, Schema, SchemaValue};
use mmapview::{query_from_generic, EvalResult};

const VERSION: &str = env!("CARGO_PKG_VERSION");

fn print_usage() {
    eprintln!("Usage:");
    eprintln!("  mmapview <data-file> <schema-file> <query>            DSL query mode");
    eprintln!("  mmapview [options] <data-file>                        Tree view mode");
    eprintln!("  mmapview --self-schema-path <output-path>             Extract self-schema");
    eprintln!();
    eprintln!("Tree mode (auto-detected on non-TTY stdout, or with --tree):");
    eprintln!("  mmapview data.mms                    auto-detect");
    eprintln!("  mmapview --tree data.mms             explicit");
    eprintln!("  mmapview --depth=5 data.mms          with options");
    eprintln!();
    eprintln!("Tree options:");
    eprintln!("  --tree           Force tree mode");
    eprintln!("  --depth=N        Max nesting depth (default: 4)");
    eprintln!("  --max-array=N    Max array elements (default: 20)");
    eprintln!("  --max-string=N   Max string length (default: 80)");
    eprintln!("  --no-values      Show structure only");
    eprintln!("  --version, -V    Print version");
    eprintln!();
    eprintln!("Query examples:");
    eprintln!("  mmapview data.mms schema.mms /");
    eprintln!("  mmapview data.mms schema.mms /field_name");
    eprintln!("  mmapview data.mms schema.mms /items[0]/nested");
    eprintln!("  mmapview data.mms schema.mms \"/items | count()\"");
    eprintln!("  mmapview data.mms schema.mms \"/items[?id>3] | select(.id=10)\"");
}

fn main() {
    let args: Vec<String> = std::env::args().collect();

    if args.iter().any(|a| a == "--version" || a == "-V") {
        println!("mmapview v{}", VERSION);
        return;
    }

    if args.len() < 2 {
        print_usage();
        process::exit(1);
    }

    if args.iter().any(|a| a == "--help" || a == "-h") {
        print_usage();
        process::exit(0);
    }

    // Handle --self-schema-path subcommand
    if args.len() >= 3 && args[1] == "--self-schema-path" {
        let path = &args[2];
        let payload = mmapstruct_schemagen::SELF_SCHEMA_MMS;
        match fs::write(path, payload) {
            Ok(_) => {
                println!("Wrote self-schema ({} bytes) to {}", payload.len(), path);
                process::exit(0);
            }
            Err(e) => {
                eprintln!("Error writing self-schema to '{}': {}", path, e);
                process::exit(1);
            }
        }
    }

    // Determine mode: tree vs query
    // Tree mode if --tree flag is present, or auto-detect on non-TTY stdout
    // (but only when there are ≤ 2 args — otherwise it's likely query mode)
    let explicit_tree = args.iter().any(|a| a == "--tree");
    let auto_tree = !std::io::IsTerminal::is_terminal(&std::io::stdout()) && args.len() <= 3;

    if explicit_tree || auto_tree {
        // Find the data file: last argument that is not a flag
        let data_path = args.iter().rev()
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
            if arg.starts_with("--depth=") {
                opts.depth = arg["--depth=".len()..].parse().unwrap_or(4);
            } else if arg.starts_with("--max-array=") {
                opts.max_array = arg["--max-array=".len()..].parse().unwrap_or(20);
            } else if arg.starts_with("--max-string=") {
                opts.max_string = arg["--max-string=".len()..].parse().unwrap_or(80);
            } else if arg == "--no-values" {
                opts.no_values = true;
            }
        }

        match mmapview::tree::format_file(&data, &opts) {
            Ok(tree) => {
                print!("{}", tree);
            }
            Err(e) => {
                eprintln!("Tree error: {}", e);
                process::exit(1);
            }
        }
        return;
    }

    // Query mode: exactly 3 positional args
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
                match r {
                    EvalResult::Buf(v) => {
                        println!("{}", format_schema_value(&ctx, v));
                    }
                    EvalResult::U64(n) => println!("{}", n),
                    EvalResult::I64(n) => println!("{}", n),
                    EvalResult::F64(n) => {
                        if *n == n.trunc() && !n.is_infinite() {
                            println!("{}", n.trunc() as i64);
                        } else {
                            println!("{}", n);
                        }
                    }
                }
            }
        }
        Err(e) => {
            eprintln!("Query error: {}", e);
            process::exit(1);
        }
    }
}
