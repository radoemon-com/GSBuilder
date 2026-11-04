mod bootstrap;
mod cli;
mod update;

use crate::core::config::Config;
use crate::core::error::AppResult;
use crate::core::language::Language;

pub fn run() -> AppResult<()> {
    let mut config = Config::load()?;
    let language = Language::load(&config.language_file(&config.language.code)?)?;
    println!("\n{}", language.text("app.title")?);
    if update::check_and_update(&config, &language)? {
        return Ok(());
    }
    bootstrap::ensure_runtime_dependencies(&mut config, &language)?;
    cli::run(&config, &language)
}
