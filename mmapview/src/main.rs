use std::fs;
use std::process;

use mmapstruct::traits::MmapRead;
use mmapstruct_schema::context::{format_schema_value, SchemaDynContext, SchemaValue};
use mmapstruct_schema::Schema;
use mmapview::{query_from_generic, EvalResult};

fn print_usage() {
    eprintln!("Usage:");
    eprintln!("  mmapview <data-file> <schema-file> <query>            DSL query mode");
    eprintln!("  mmapview <data-file> --tree [options]                 Tree view mode");
    eprintln!("  mmapview --self-schema-path <output-path>             Extract self-schema");
    eprintln!();
    eprintln!("Tree options:");
    eprintln!("  --depth=N        Max nesting depth (default: 4)");
    eprintln!("  --max-array=N    Max array elements (default: 20)");
    eprintln!("  --max-string=N   Max string length (default: 80)");
    eprintln!("  --no-values      Show structure only");
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

    if args.len() < 2 {
        print_usage();
        process::exit(1);
    }

    if args[1] == "--help" || args[1] == "-h" {
        print_usage();
        process::exit(0);
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
                process::exit(0);
            }
            Err(e) => {
                eprintln!("Error writing self-schema to '{}': {}", path, e);
                process::exit(1);
            }
        }
    }

    // Tree mode detection: --tree flag, or auto-detect if stdout is not a terminal
    let is_tree = args.iter().any(|a| a == "--tree")
        || (!std::io::IsTerminal::is_terminal(&std::io::stdout()) && args.len() <= 4);

    if is_tree {
        let data_path = &args[1];
        let data = match fs::read(data_path) {
            Ok(d) => d,
            Err(e) => {
                eprintln!("Error reading data file '{}': {}", data_path, e);
                process::exit(1);
            }
        };

        let mut opts = mmapview::tree::TreeOptions::default();
        // Parse optional arguments
        for arg in &args[2..] {
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

    // Query mode (existing behavior)
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

    let schema_reader = <Schema as MmapRead>::read(&schema_bytes, 0..schema_bytes.len());
    let root_type = *schema_reader.root_type;

    let ctx = SchemaDynContext::new(&data, &schema_bytes);
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
