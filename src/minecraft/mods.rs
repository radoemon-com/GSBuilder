use crate::core::config::Config;
use crate::core::error::{AppResult, input_error};
use crate::core::files::copy_path;
use crate::core::language::Language;
use crate::core::paths::is_valid_component;
use std::path::Path;

pub(super) fn add_to_server(
    config: &Config,
    language: &Language,
    server_name: &str,
    source: &Path,
) -> AppResult<()> {
    if !is_valid_component(server_name) {
        return Err(input_error(language.text("paths.component.invalid")?));
    }
    if !source.exists() {
        return Err(input_error(format!(
            "Mod source directory does not exist: {}",
            source.display()
        )));
    }
    let server = config.minecraft_servers_path().join(server_name);
    if !server.is_dir() {
        return Err(input_error(format!(
            "Minecraft server does not exist: {}",
            server.display()
        )));
    }
    let destination = server.join("mods");
    copy_path(source, &destination)?;
    println!(
        "{}",
        language.format(
            "cli.mods.copied",
            &[("path", &destination.display().to_string())]
        )?
    );
    Ok(())
}
