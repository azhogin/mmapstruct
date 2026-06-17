use std::fs;
use std::process;

use mmapstruct::traits::MmapRead;
use mmapstruct_schema::context::{format_schema_value, SchemaDynContext, SchemaValue};
use mmapstruct_schema::Schema;
use mmapview::{query_from_generic, EvalResult};

fn main() {
    let args: Vec<String> = std::env::args().collect();

    // Handle --self-schema-path subcommand: extract embedded self-schema
    if args.len() >= 3 && args[1] == "--self-schema-path" {
        let path = &args[2];
        match fs::write(path, mmapstruct_schemagen::SELF_SCHEMA_MMS) {
            Ok(_) => {
                println!("Wrote self-schema ({} bytes) to {}", mmapstruct_schemagen::SELF_SCHEMA_MMS.len(), path);
                process::exit(0);
            }
            Err(e) => {
                eprintln!("Error writing self-schema to '{}': {}", path, e);
                process::exit(1);
            }
        }
    }

    if args.len() < 4 {
        eprintln!("Usage:");
        eprintln!("  mmapview <data-file> <schema-file> <query>");
        eprintln!("  mmapview --self-schema-path <output-path>");
        eprintln!();
        eprintln!("Query examples:");
        eprintln!("  mmapview data.mms schema.mms /");
        eprintln!("  mmapview data.mms schema.mms /field_name");
        eprintln!("  mmapview data.mms schema.mms /items[0]/nested");
        eprintln!("  mmapview data.mms schema.mms \"/items | count()\"");
        eprintln!("  mmapview data.mms schema.mms \"/items[?id>3] | select(.id=10)\"");
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
