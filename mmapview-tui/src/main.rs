use std::fs;
use std::io;
use std::path::Path;

use crossterm::terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen};
use crossterm::ExecutableCommand;
use ratatui::backend::CrosstermBackend;
use ratatui::Terminal;

use mmapview_tui::app::App;
use mmapview_tui::input;
use mmapview_tui::schema;
use mmapview_tui::scanner::Scanner;
use mmapview_tui::overscan::Overscan;
use mmapview_tui::ui;

const VERSION: &str = env!("CARGO_PKG_VERSION");

fn print_usage() {
    eprintln!("Usage:");
    eprintln!("  mmapview-tui [options] <data-file>");
    eprintln!();
    eprintln!("Options:");
    eprintln!("  --schema <file>     Schema file (if not embedded)");
    eprintln!("  --version, -V       Print version");
    eprintln!("  --help              Print this help");
}

fn main() {
    let args: Vec<String> = std::env::args().collect();

    if args.iter().any(|a| a == "--version" || a == "-V") {
        println!("mmapview-tui v{}", VERSION);
        return;
    }

    if args.len() < 2 || args.iter().any(|a| a == "--help" || a == "-h") {
        print_usage();
        std::process::exit(if args.len() < 2 { 1 } else { 0 });
    }

    let mut schema_path: Option<String> = None;
    let mut data_path: Option<String> = None;

    let mut i = 1;
    while i < args.len() {
        let arg = &args[i];
        if arg == "--schema" && i + 1 < args.len() {
            schema_path = Some(args[i + 1].clone());
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

    let data_vec = match fs::read(&data_path) {
        Ok(d) => d,
        Err(e) => {
            eprintln!("Error reading '{}': {}", data_path, e);
            std::process::exit(1);
        }
    };

    let loaded = match schema::load_schema(&data_vec, schema_path.as_deref(), Some(&data_path)) {
        Ok(l) => l,
        Err(e) => {
            eprintln!("{}", e);
            std::process::exit(1);
        }
    };

    // Leak data & ctx to 'static so App doesn't need a lifetime parameter.
    let data: &'static [u8] = Box::leak(data_vec.into_boxed_slice());
    let schema_bytes: &'static [u8] = Box::leak(loaded.schema_bytes.into_boxed_slice());
    let ctx: &'static mmapstruct_schema::context::RuntimeSchemaCtx<'static> = Box::leak(Box::new(
        mmapstruct_schema::context::RuntimeSchemaCtx::new(data, schema_bytes)
    ));

    let schema_reader = <mmapstruct_schema::Schema as mmapstruct::traits::MmapRead>::read(
        schema_bytes,
        0..schema_bytes.len(),
    );
    let root_type = *schema_reader.root_type;

    let root = mmapstruct_schema::SchemaValue::new(
        loaded.payload_range.start as u64,
        loaded.payload_range.end as u64,
        root_type,
    );

    // Build new-style App (Overscan + Viewport)
    let scanner = Scanner::new(ctx, root, 100);
    let overscan = Overscan::new(scanner);
    let term_height = crossterm::terminal::size()
        .map(|(_, h)| h as usize)
        .unwrap_or(40);
    let mut app = App::new(overscan, term_height);

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
    app: &mut App,
) -> Result<(), String> {
    loop {
        terminal.draw(|f| ui::draw_viewport(f, &app.viewport, &app.overscan, app.search_mode, &app.search_buffer, app.search_message))
            .map_err(|e| e.to_string())?;
        if !input::handle_input(app)? {
            break Ok(());
        }
    }
}
