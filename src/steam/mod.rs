mod mods;
mod workshop;

use crate::core::config::Config;
use crate::core::error::{AppResult, input_error};
use crate::core::language::Language;
use crate::core::paths::{is_valid_component, prepare_target};
use std::io::{self, Write};
use std::path::Path;
use std::process::Command;

pub struct SteamService<'a> {
    config: &'a Config,
    language: &'a Language,
}

impl<'a> SteamService<'a> {
    pub fn new(config: &'a Config, language: &'a Language) -> Self {
        Self { config, language }
    }

    pub fn install(&self, game: &str, server: &str, app_id: &str) -> AppResult<()> {
        if !is_valid_component(game) || !is_valid_component(server) {
            return Err(input_error(self.language.text("paths.component.invalid")?));
        }
        if app_id.is_empty() || !app_id.bytes().all(|byte| byte.is_ascii_digit()) {
            return Err(input_error(self.language.text("steam.app_id.invalid")?));
        }
        let steam_username = match self.config.steam.login_mode.as_str() {
            "anonymous" => None,
            "account" => Some(prompt_for_steam_username(self.language)?),
            mode => {
                return Err(input_error(format!("Unsupported Steam login mode: {mode}")));
            }
        };
        let target = self.config.steam_servers_path(game)?.join(server);
        prepare_install_target(&target, &self.config.steam.run_as_user)?;
        let steamcmd = self
            .config
            .executable_path(&self.config.steam.steamcmd_executable);
        let mut args = vec![
            "+force_install_dir".to_owned(),
            target.to_string_lossy().into_owned(),
            "+login".to_owned(),
            steam_username.unwrap_or_else(|| "anonymous".to_owned()),
            "+app_update".to_owned(),
            app_id.to_owned(),
        ];
        if self.config.steam.validate_install {
            args.push("validate".to_owned());
        }
        args.push("+quit".to_owned());
        let status = run_steamcmd(&steamcmd, &args, &self.config.steam.run_as_user)?;
        if !status.success() {
            return Err(input_error(self.language.format(
                "steam.install.failed",
                &[("status", &status.to_string())],
            )?));
        }
        println!(
            "{} {}",
            self.language.text("server.ready")?,
            target.display()
        );
        Ok(())
    }

    pub fn add_mods(
        &self,
        game: &str,
        server: &str,
        source: &Path,
        destination: Option<&str>,
    ) -> AppResult<()> {
        let server_path = self.server_path(game, server)?;
        mods::add(self.language, &server_path, source, destination)
    }

    pub fn list_mods(&self, game: &str, server: &str, destination: Option<&str>) -> AppResult<()> {
        let server_path = self.server_path(game, server)?;
        mods::list(self.language, &server_path, destination)
    }

    pub fn install_workshop_item(
        &self,
        game: &str,
        server: &str,
        workshop_id: &str,
    ) -> AppResult<()> {
        let server_path = self.server_path(game, server)?;
        workshop::install_item(self.config, self.language, &server_path, workshop_id)
    }

    pub fn install_workshop_collection(
        &self,
        game: &str,
        server: &str,
        collection_id: &str,
    ) -> AppResult<()> {
        let server_path = self.server_path(game, server)?;
        workshop::install_collection(self.config, self.language, &server_path, collection_id)
    }

    pub fn remove_workshop_item(
        &self,
        game: &str,
        server: &str,
        workshop_id: &str,
    ) -> AppResult<()> {
        let server_path = self.server_path(game, server)?;
        workshop::remove_item(self.language, &server_path, workshop_id)
    }

    pub fn remove_workshop_collection(
        &self,
        game: &str,
        server: &str,
        collection_id: &str,
    ) -> AppResult<()> {
        let server_path = self.server_path(game, server)?;
        workshop::remove_collection(self.config, self.language, &server_path, collection_id)
    }

    fn server_path(&self, game: &str, server: &str) -> AppResult<std::path::PathBuf> {
        if !is_valid_component(game) || !is_valid_component(server) {
            return Err(input_error(self.language.text("paths.component.invalid")?));
        }
        let path = self.config.steam_servers_path(game)?.join(server);
        if !path.is_dir() {
            return Err(input_error(format!(
                "Steam server does not exist: {}",
                path.display()
            )));
        }
        Ok(path)
    }
}

fn prepare_install_target(target: &Path, run_as_user: &str) -> AppResult<()> {
    if run_as_user.is_empty() {
        return prepare_target(target);
    }

    prepare_target(target)?;
    let status = Command::new("sudo")
        .args(["chown", "--", run_as_user])
        .arg(target)
        .status()?;
    if !status.success() {
        return Err(input_error(format!(
            "Could not assign Steam install directory to {run_as_user}: {status}"
        )));
    }
    Ok(())
}

pub(super) fn prompt_for_steam_username(language: &Language) -> AppResult<String> {
    print!("{}", language.text("steam.username.prompt")?);
    io::stdout().flush()?;
    let mut username = String::new();
    if io::stdin().read_line(&mut username)? == 0 {
        return Err(input_error(language.text("steam.username.required")?));
    }
    let username = username.trim();
    if username.is_empty() || username.chars().any(char::is_control) {
        return Err(input_error(language.text("steam.username.required")?));
    }
    Ok(username.to_owned())
}

pub(super) fn run_steamcmd(
    executable: &Path,
    args: &[String],
    run_as_user: &str,
) -> AppResult<std::process::ExitStatus> {
    if run_as_user.is_empty() {
        return Ok(Command::new(executable).args(args).status()?);
    }

    #[cfg(unix)]
    {
        let status = Command::new("sudo")
            .arg("-u")
            .arg(run_as_user)
            .arg("--")
            .arg(executable)
            .args(args)
            .status()?;
        Ok(status)
    }

    #[cfg(windows)]
    {
        let command_line = std::iter::once(executable.to_string_lossy().into_owned())
            .chain(args.iter().cloned())
            .map(|arg| quote_windows_argument(&arg))
            .collect::<Vec<_>>()
            .join(" ");
        Ok(Command::new("runas")
            .arg(format!("/user:{run_as_user}"))
            .arg(command_line)
            .status()?)
    }

    #[cfg(not(any(unix, windows)))]
    {
        let _ = (executable, args);
        Err(input_error(
            "Running SteamCMD as another OS user is unsupported on this platform",
        ))
    }
}

#[cfg(windows)]
fn quote_windows_argument(argument: &str) -> String {
    let mut quoted = String::from("\"");
    let mut backslashes = 0;
    for character in argument.chars() {
        match character {
            '\\' => backslashes += 1,
            '"' => {
                quoted.extend(std::iter::repeat_n('\\', backslashes * 2 + 1));
                quoted.push('"');
                backslashes = 0;
            }
            _ => {
                quoted.extend(std::iter::repeat_n('\\', backslashes));
                quoted.push(character);
                backslashes = 0;
            }
        }
    }
    quoted.extend(std::iter::repeat_n('\\', backslashes * 2));
    quoted.push('"');
    quoted
}
