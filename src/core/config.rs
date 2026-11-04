use crate::core::error::{AppResult, input_error};
use serde::Deserialize;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Component, Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};

#[derive(Debug, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Config {
    pub language: LanguageConfig,
    pub paths: PathsConfig,
    pub minecraft: MinecraftConfig,
    pub steam: SteamConfig,
    pub network: NetworkConfig,
    pub updates: UpdatesConfig,
    config_directory: PathBuf,
    config_path: PathBuf,
}

#[derive(Debug, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct LanguageConfig {
    pub code: String,
    pub japanese_file: PathBuf,
    pub english_file: PathBuf,
}

#[derive(Debug, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct PathsConfig {
    pub minecraft_servers: PathBuf,
    pub steam_servers: PathBuf,
}

#[derive(Debug, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct MinecraftConfig {
    pub java_executable: PathBuf,
    pub minimum_memory: String,
    pub maximum_memory: String,
    pub api_registry: PathBuf,
}

#[derive(Debug, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct SteamConfig {
    pub steamcmd_executable: PathBuf,
    pub validate_install: bool,
    pub run_as_user: String,
    pub login_mode: String,
}

#[derive(Debug, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct NetworkConfig {
    pub timeout_seconds: u64,
    pub user_agent: String,
}

#[derive(Debug, Deserialize, Default)]
#[serde(default, deny_unknown_fields)]
pub struct UpdatesConfig {
    pub github_repository: String,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            language: LanguageConfig::default(),
            paths: PathsConfig::default(),
            minecraft: MinecraftConfig::default(),
            steam: SteamConfig::default(),
            network: NetworkConfig::default(),
            updates: UpdatesConfig::default(),
            config_directory: PathBuf::from("."),
            config_path: PathBuf::from("config.toml"),
        }
    }
}

impl Default for LanguageConfig {
    fn default() -> Self {
        Self {
            code: "ja".to_owned(),
            japanese_file: PathBuf::from("lang/ja.json"),
            english_file: PathBuf::from("lang/en.json"),
        }
    }
}

impl Default for PathsConfig {
    fn default() -> Self {
        Self {
            minecraft_servers: PathBuf::from("servers/Minecraft"),
            steam_servers: PathBuf::from("servers/{game}"),
        }
    }
}

impl Default for MinecraftConfig {
    fn default() -> Self {
        Self {
            java_executable: PathBuf::from("java"),
            minimum_memory: "1G".to_owned(),
            maximum_memory: "2G".to_owned(),
            api_registry: PathBuf::from("data/minecraft/apis.json"),
        }
    }
}

impl Default for SteamConfig {
    fn default() -> Self {
        Self {
            steamcmd_executable: PathBuf::from(if cfg!(windows) {
                "steamcmd.exe"
            } else {
                "steamcmd"
            }),
            validate_install: true,
            run_as_user: String::new(),
            login_mode: "anonymous".to_owned(),
        }
    }
}

impl Default for NetworkConfig {
    fn default() -> Self {
        Self {
            timeout_seconds: 120,
            user_agent: "GSBuilder/1.0".to_owned(),
        }
    }
}

impl Config {
    pub fn load() -> AppResult<Self> {
        let config_path = find_config_file()?;
        let config_directory = config_path
            .parent()
            .unwrap_or_else(|| Path::new("."))
            .to_path_buf();
        let contents = fs::read_to_string(&config_path).map_err(|error| {
            input_error(format!(
                "Could not read config file {}: {error}",
                config_path.display()
            ))
        })?;
        Self::parse_contents(&contents, config_directory, config_path.clone()).map_err(|error| {
            input_error(format!(
                "Invalid config file {}: {error}",
                config_path.display()
            ))
        })
    }

    fn parse_contents(
        contents: &str,
        config_directory: PathBuf,
        config_path: PathBuf,
    ) -> AppResult<Self> {
        let mut config: Self = toml::from_str(contents)
            .map_err(|error| input_error(format!("Invalid configuration: {error}")))?;
        config.config_directory = config_directory;
        config.config_path = config_path;
        config.validate()?;
        Ok(config)
    }

    #[cfg(test)]
    pub(crate) fn from_contents(contents: &str, config_directory: PathBuf) -> AppResult<Self> {
        let config_path = config_directory.join("config.toml");
        Self::parse_contents(contents, config_directory, config_path)
    }

    fn validate(&self) -> AppResult<()> {
        if !matches!(self.language.code.as_str(), "ja" | "en") {
            return Err(input_error(format!(
                "Unsupported language code {:?}; use \"ja\" or \"en\"",
                self.language.code
            )));
        }
        if self.network.timeout_seconds == 0 {
            return Err(input_error(
                "network.timeout_seconds must be greater than zero",
            ));
        }
        if self.network.user_agent.trim().is_empty() {
            return Err(input_error("network.user_agent cannot be empty"));
        }
        if !self.updates.github_repository.is_empty()
            && (self.updates.github_repository.split('/').count() != 2
                || !self.updates.github_repository.split('/').all(|part| {
                    !part.is_empty()
                        && part.bytes().all(|byte| {
                            byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.')
                        })
                }))
        {
            return Err(input_error(
                "updates.github_repository must be empty or use the owner/repository format",
            ));
        }
        if !matches!(self.steam.login_mode.as_str(), "anonymous" | "account") {
            return Err(input_error(
                "steam.login_mode must be \"anonymous\" or \"account\"",
            ));
        }
        if self.steam.run_as_user.chars().any(char::is_control) {
            return Err(input_error(
                "steam.run_as_user cannot contain control characters",
            ));
        }
        let minimum = memory_megabytes("minecraft.minimum_memory", &self.minecraft.minimum_memory)?;
        let maximum = memory_megabytes("minecraft.maximum_memory", &self.minecraft.maximum_memory)?;
        if minimum > maximum {
            return Err(input_error(
                "minecraft.minimum_memory cannot exceed minecraft.maximum_memory",
            ));
        }
        Ok(())
    }

    pub fn language_file(&self, code: &str) -> AppResult<PathBuf> {
        let configured_path = match code {
            "ja" => &self.language.japanese_file,
            "en" => &self.language.english_file,
            _ => return Err(input_error(format!("Unsupported language code: {code}"))),
        };
        Ok(self.resolve_path(configured_path))
    }

    pub fn minecraft_servers_path(&self) -> PathBuf {
        self.resolve_path(&self.paths.minecraft_servers)
    }

    pub fn minecraft_api_registry_path(&self) -> PathBuf {
        self.resolve_path(&self.minecraft.api_registry)
    }

    pub fn steam_servers_path(&self, game: &str) -> AppResult<PathBuf> {
        let template = self.paths.steam_servers.to_string_lossy();
        if template.contains("{game}") {
            Ok(self.resolve_path(Path::new(&template.replace("{game}", game))))
        } else {
            Err(input_error(
                "paths.steam_servers must include the {game} placeholder",
            ))
        }
    }

    pub fn executable_path(&self, configured: &Path) -> PathBuf {
        if configured.is_absolute()
            || configured
                .components()
                .any(|component| matches!(component, Component::CurDir | Component::ParentDir))
            || configured.components().count() > 1
        {
            self.resolve_path(configured)
        } else {
            configured.to_path_buf()
        }
    }

    pub fn save_steam_install(&mut self, executable: &Path, run_as_user: &str) -> AppResult<()> {
        let mut document = fs::read_to_string(&self.config_path)
            .map_err(|error| input_error(format!("Could not read config file: {error}")))?
            .parse::<toml_edit::DocumentMut>()
            .map_err(|error| input_error(format!("Could not parse config file: {error}")))?;
        let steam = document
            .get_mut("steam")
            .and_then(toml_edit::Item::as_table_mut)
            .ok_or_else(|| input_error("config.toml is missing the [steam] section"))?;
        steam["steamcmd_executable"] = toml_edit::value(executable.to_string_lossy().as_ref());
        steam["run_as_user"] = toml_edit::value(run_as_user);
        let contents = document.to_string();
        let permissions = fs::metadata(&self.config_path)?.permissions();
        let temp_path = temporary_config_path(&self.config_path);
        let write_result = (|| {
            let mut file = OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&temp_path)?;
            file.write_all(contents.as_bytes())?;
            file.sync_all()?;
            fs::set_permissions(&temp_path, permissions)?;
            fs::rename(&temp_path, &self.config_path)?;
            Ok::<_, std::io::Error>(())
        })();
        if let Err(error) = write_result {
            if temp_path.exists() {
                fs::remove_file(&temp_path)?;
            }
            return Err(input_error(format!(
                "Could not safely update config file: {error}"
            )));
        }
        self.steam.steamcmd_executable = executable.to_path_buf();
        self.steam.run_as_user = run_as_user.to_owned();
        Ok(())
    }

    fn resolve_path(&self, path: &Path) -> PathBuf {
        if path.is_absolute() {
            path.to_path_buf()
        } else {
            self.config_directory.join(path)
        }
    }
}

fn temporary_config_path(config_path: &Path) -> PathBuf {
    static NEXT_ID: AtomicUsize = AtomicUsize::new(0);
    let file_name = config_path
        .file_name()
        .unwrap_or_default()
        .to_string_lossy();
    config_path.with_file_name(format!(
        ".{file_name}.{}.{}.tmp",
        std::process::id(),
        NEXT_ID.fetch_add(1, Ordering::Relaxed)
    ))
}

fn memory_megabytes(key: &str, value: &str) -> AppResult<u128> {
    let Some(unit) = value.chars().last() else {
        return Err(input_error(format!("{key} cannot be empty")));
    };
    let amount = &value[..value.len() - unit.len_utf8()];
    let amount = amount
        .parse::<u128>()
        .map_err(|_| input_error(format!("{key} must start with a positive number")))?;
    if !matches!(unit, 'M' | 'G') || amount == 0 {
        return Err(input_error(format!(
            "{key} must be a positive number followed by M or G (for example 2G)"
        )));
    }
    if unit == 'G' {
        amount
            .checked_mul(1024)
            .ok_or_else(|| input_error(format!("{key} is too large")))
    } else {
        Ok(amount)
    }
}

fn find_config_file() -> AppResult<PathBuf> {
    if let Some(path) = std::env::var_os("GSBUILDER_CONFIG") {
        let path = PathBuf::from(path);
        if path.is_file() {
            return Ok(path);
        }
        return Err(input_error(format!(
            "GSBUILDER_CONFIG does not point to a file: {}",
            path.display()
        )));
    }

    let mut candidates = Vec::new();
    if let Ok(executable) = std::env::current_exe()
        && let Some(parent) = executable.parent()
    {
        candidates.push(parent.join("config.toml"));
    }
    if let Ok(current_dir) = std::env::current_dir() {
        candidates.push(current_dir.join("config.toml"));
    }
    candidates.push(Path::new(env!("CARGO_MANIFEST_DIR")).join("config.toml"));

    candidates
        .into_iter()
        .find(|path| path.is_file())
        .ok_or_else(|| input_error("Could not find config.toml"))
}

#[cfg(test)]
mod tests {
    use super::{Config, memory_megabytes};
    use std::fs;
    use std::path::Path;

    #[test]
    fn defaults_cover_language_paths_and_programs() {
        let config = Config::default();
        assert_eq!(config.language.code, "ja");
        assert_eq!(
            config.paths.minecraft_servers,
            Path::new("servers/Minecraft")
        );
        assert_eq!(config.paths.steam_servers, Path::new("servers/{game}"));
        assert_eq!(config.minecraft.minimum_memory, "1G");
        assert_eq!(config.minecraft.maximum_memory, "2G");
        assert_eq!(
            config.minecraft.api_registry,
            Path::new("data/minecraft/apis.json")
        );
        assert!(config.steam.validate_install);
        assert!(config.steam.run_as_user.is_empty());
        assert_eq!(config.steam.login_mode, "anonymous");
        assert!(config.updates.github_repository.is_empty());
    }

    #[test]
    fn validates_optional_github_update_repository() {
        let config = Config::from_contents(
            "[updates]\ngithub_repository = \"owner/project\"\n",
            Path::new(".").to_path_buf(),
        )
        .unwrap();
        assert_eq!(config.updates.github_repository, "owner/project");
        assert!(
            Config::from_contents(
                "[updates]\ngithub_repository = \"https://github.com/owner/project\"\n",
                Path::new(".").to_path_buf(),
            )
            .is_err()
        );
    }

    #[test]
    fn loads_repository_config_file() {
        let config = Config::load().unwrap();
        assert_eq!(config.language.code, "ja");
        assert_eq!(config.network.timeout_seconds, 120);
        assert!(config.language_file("ja").unwrap().is_file());
        assert!(config.language_file("en").unwrap().is_file());
    }

    #[test]
    fn rejects_invalid_memory_values() {
        assert_eq!(memory_megabytes("memory", "2G").unwrap(), 2048);
        assert_eq!(memory_megabytes("memory", "512M").unwrap(), 512);
        assert!(memory_megabytes("memory", "0G").is_err());
        assert!(memory_megabytes("memory", "-1G").is_err());
        assert!(memory_megabytes("memory", "2T").is_err());
    }

    #[test]
    fn validates_steam_login_mode_and_accepts_a_run_as_user() {
        let invalid = Config::from_contents(
            "[steam]\nlogin_mode = \"invalid\"\n",
            Path::new(".").to_path_buf(),
        );
        assert!(invalid.is_err());

        let configured = Config::from_contents(
            "[steam]\nlogin_mode = \"account\"\nrun_as_user = \"game-user\"\n",
            Path::new(".").to_path_buf(),
        )
        .unwrap();
        assert_eq!(configured.steam.login_mode, "account");
        assert_eq!(configured.steam.run_as_user, "game-user");
    }

    #[test]
    fn persists_installed_steamcmd_path_and_user_without_dropping_config_comments() {
        let directory =
            std::env::temp_dir().join(format!("gsbuilder-config-save-{}", std::process::id()));
        fs::create_dir_all(&directory).unwrap();
        let config_path = directory.join("config.toml");
        fs::write(
            &config_path,
            "# Keep user comments\n[steam]\nsteamcmd_executable = \"steamcmd\"\nrun_as_user = \"\"\n",
        )
        .unwrap();
        let mut config = Config::from_contents(
            &fs::read_to_string(&config_path).unwrap(),
            directory.clone(),
        )
        .unwrap();
        let executable = directory.join("user/.local/share/gsbuilder/steamcmd/steamcmd.sh");

        config.save_steam_install(&executable, "game-user").unwrap();

        let saved = fs::read_to_string(&config_path).unwrap();
        assert!(saved.contains("# Keep user comments"));
        assert!(saved.contains(&executable.display().to_string()));
        assert!(saved.contains("run_as_user = \"game-user\""));
        assert_eq!(config.steam.steamcmd_executable, executable);
        assert_eq!(config.steam.run_as_user, "game-user");
        fs::remove_dir_all(directory).unwrap();
    }
}
