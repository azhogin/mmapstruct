use std::fs;
use std::io;
use std::path::Path;

use crossterm::terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen};
use crossterm::ExecutableCommand;
use ratatui::backend::CrosstermBackend;
use ratatui::Terminal;

use mmapview_tui::app;
use mmapview_tui::input;
use mmapview_tui::schema;
use mmapview_tui::tree;
use mmapview_tui::ui;

fn print_usage() {
    eprintln!("Usage:");
    eprintln!("  mmapview-tui [options] <data-file>");
    eprintln!();
    eprintln!("Options:");
    eprintln!("  --schema <file>     Schema file (if not embedded)");
    eprintln!("  --depth <N>         Initial expand depth (default: 1)");
    eprintln!("  --help              Print this help");
}

fn main() {
    let args: Vec<String> = std::env::args().collect();

    if args.len() < 2 || args.iter().any(|a| a == "--help" || a == "-h") {
        print_usage();
        std::process::exit(if args.len() < 2 { 1 } else { 0 });
    }

    // Parse options — they may appear before or after the filename.
    let mut schema_path: Option<String> = None;
    let mut depth = 1;
    let mut data_path: Option<String> = None;

    let mut i = 1;
    while i < args.len() {
        let arg = &args[i];
        if arg == "--schema" && i + 1 < args.len() {
            schema_path = Some(args[i + 1].clone());
            i += 2;
        } else if let Some(val) = arg.strip_prefix("--depth=") {
            depth = val.parse().unwrap_or(1);
            i += 1;
        } else if arg == "--depth" && i + 1 < args.len() {
            depth = args[i + 1].parse().unwrap_or(1);
            i += 2;
        } else if arg.starts_with("--") {
            eprintln!("Unknown option: {}", arg);
            std::process::exit(1);
        } else if data_path.is_none() {
            data_path = Some(arg.clone());
            i += 1;
        } else {
            eprintln!("Unexpected extra argument: {}", arg);
            std::process::exit(1);
        }
    }

    let data_path = match data_path {
        Some(p) if Path::new(&p).exists() => p,
        _ => {
            eprintln!("Error: data file not found or not specified.");
            print_usage();
            std::process::exit(1);
        }
    };

    let data = match fs::read(&data_path) {
        Ok(d) => d,
        Err(e) => {
            eprintln!("Error reading '{}': {}", data_path, e);
            std::process::exit(1);
        }
    };

    let loaded = match schema::load_schema(&data, schema_path.as_deref()) {
        Ok(l) => l,
        Err(e) => {
            eprintln!("{}", e);
            std::process::exit(1);
        }
    };

    let ctx = mmapstruct_schema::context::RuntimeSchemaCtx::new(&data, &loaded.schema_bytes);
    let schema_reader = <mmapstruct_schema::Schema as mmapstruct::traits::MmapRead>::read(
        &loaded.schema_bytes,
        0..loaded.schema_bytes.len(),
    );
    let root_type = *schema_reader.root_type;

    let root = mmapstruct_schema::SchemaValue::new(
        loaded.payload_range.start as u64,
        loaded.payload_range.end as u64,
        root_type,
    );

    let type_info = ctx.type_reader(root_type);
    let root_name = match type_info {
        mmapstruct_schema::TypeDefReader::Struct { name, .. } => name.to_string(),
        mmapstruct_schema::TypeDefReader::Enum { name, .. } => name.to_string(),
        _ => "Root".to_string(),
    };

    let nodes = tree::build_tree(&ctx, &root, &root_name, depth);
    let mut app = app::App::new(nodes, depth);

    // Setup terminal
    enable_raw_mode().expect("raw mode");
    let mut stdout = io::stdout();
    stdout.execute(EnterAlternateScreen).expect("alternate screen");
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend).expect("terminal");

    let result = run_tui(&mut terminal, &mut app);

    // Restore terminal
    disable_raw_mode().expect("raw mode off");
    let mut stdout = io::stdout();
    stdout.execute(LeaveAlternateScreen).expect("leave alternate screen");
    crossterm::execute!(std::io::stdout(), crossterm::cursor::Show).ok();

    if let Err(e) = result {
        eprintln!("Error: {}", e);
        std::process::exit(1);
    }
}

fn run_tui(
    terminal: &mut Terminal<CrosstermBackend<impl io::Write>>,
    app: &mut app::App,
) -> Result<(), String> {
    loop {
        terminal.draw(|f| ui::draw(f, app)).map_err(|e| e.to_string())?;
        if !input::handle_input(app)? {
            break Ok(());
        }
    }
}
