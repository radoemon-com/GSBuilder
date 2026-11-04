mod app;
mod core;
mod minecraft;
mod steam;

fn main() {
    if let Err(error) = app::run() {
        eprintln!("Error: {error}");
        std::process::exit(1);
    }
}
