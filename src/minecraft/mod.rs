pub mod api;
mod installer;
mod mods;

use crate::core::config::Config;
use crate::core::error::{AppResult, input_error};
use crate::core::language::Language;
use crate::core::paths::{is_valid_component, prepare_target};
use crate::core::servers::remove_server;
use installer::ServerKind;
use std::fs;
use std::path::Path;

pub struct MinecraftService<'a> {
    config: &'a Config,
    language: &'a Language,
}

impl<'a> MinecraftService<'a> {
    pub fn new(config: &'a Config, language: &'a Language) -> Self {
        Self { config, language }
    }

    pub fn list_apis(&self) -> AppResult<()> {
        let registry = api::MinecraftApiRegistry::load(self.config)?;
        for item in registry.apis {
            let kind = item.provider_type();
            println!("{}\t{}\t{kind}", item.id, item.name);
        }
        Ok(())
    }

    pub fn add_api(
        &self,
        provider_type: &str,
        id: &str,
        name: &str,
        endpoint_urls: &[String],
    ) -> AppResult<()> {
        let mut registry = api::MinecraftApiRegistry::load(self.config)?;
        registry.add_provider(self.config, provider_type, id, name, endpoint_urls)?;
        println!(
            "{}",
            self.language
                .format("cli.minecraft.api.added", &[("id", id)])?
        );
        Ok(())
    }

    pub fn remove_api(&self, id: &str) -> AppResult<()> {
        let mut registry = api::MinecraftApiRegistry::load(self.config)?;
        registry.remove(self.config, id)?;
        println!(
            "{}",
            self.language
                .format("cli.minecraft.api.removed", &[("id", id)])?
        );
        Ok(())
    }

    pub fn list_versions(&self, api_id: &str) -> AppResult<()> {
        let registry = api::MinecraftApiRegistry::load(self.config)?;
        let provider = registry.get(api_id)?;
        for version in api::fetch_versions(self.config, provider)? {
            println!("{version}");
        }
        Ok(())
    }

    pub fn list_fabric_loaders(&self, api_id: &str, minecraft_version: &str) -> AppResult<()> {
        let registry = api::MinecraftApiRegistry::load(self.config)?;
        let provider = registry.get(api_id)?;
        for version in api::fetch_fabric_loaders(self.config, provider, minecraft_version)? {
            println!("{version}");
        }
        Ok(())
    }

    pub fn create(
        &self,
        kind: &str,
        version: &str,
        server_name: &str,
        api_id: Option<&str>,
    ) -> AppResult<()> {
        if !is_valid_component(server_name) {
            return Err(input_error(self.language.text("paths.component.invalid")?));
        }
        if !is_valid_version(version) {
            return Err(input_error(
                self.language.text("minecraft.version.invalid")?,
            ));
        }
        let server_kind = ServerKind::parse(kind)?;
        let registry = api::MinecraftApiRegistry::load(self.config)?;
        let api_id = api_id.unwrap_or(server_kind.default_api_id());
        let api = registry.get(api_id)?.clone();
        if api.provider_type() != server_kind.provider_type() {
            return Err(input_error(format!(
                "API {} is a {} API, not a {} API",
                api.id,
                api.provider_type(),
                server_kind.provider_type()
            )));
        }
        let target = self.config.minecraft_servers_path().join(server_name);
        prepare_minecraft_target(&target, server_kind)?;

        let install_output = installer::install(
            self.config,
            &target,
            version,
            server_kind,
            &api,
            self.language,
        )?;
        fs::write(
            target.join("eula.txt"),
            "# Read https://aka.ms/MinecraftEULA and set eula=true to accept.\neula=false\n",
        )?;
        write_launchers(
            self.config,
            &target,
            install_output.jar_name.as_deref(),
            install_output.uses_generated_script,
        )?;
        println!(
            "{} {}",
            self.language.text("server.ready")?,
            target.display()
        );
        Ok(())
    }

    pub fn add_mods(&self, server_name: &str, source: &Path) -> AppResult<()> {
        mods::add_to_server(self.config, self.language, server_name, source)
    }

    pub fn remove_server(&self, server_name: &str) -> AppResult<()> {
        if !is_valid_component(server_name) {
            return Err(input_error(self.language.text("paths.component.invalid")?));
        }
        let path = remove_server(&self.config.minecraft_servers_path(), server_name)?;
        println!(
            "{}",
            self.language
                .format("server.deleted", &[("path", &path.display().to_string())])?
        );
        Ok(())
    }
}

fn prepare_minecraft_target(target: &Path, kind: ServerKind) -> AppResult<()> {
    if !target.exists() {
        return prepare_target(target);
    }

    if !target.is_dir() {
        return Err(input_error(format!(
            "Install directory must be a directory: {}",
            target.display()
        )));
    }

    let entries = fs::read_dir(target)?.collect::<Result<Vec<_>, _>>()?;
    if entries.is_empty() {
        return Ok(());
    }

    let is_fabric_installer_only = matches!(kind, ServerKind::Fabric)
        && entries.len() == 1
        && entries[0].file_name() == "fabric-installer.jar"
        && entries[0].file_type()?.is_file();
    if is_fabric_installer_only {
        return Ok(());
    }

    Err(input_error(format!(
        "Install directory must be empty or contain only a leftover Fabric installer: {}",
        target.display()
    )))
}

fn is_valid_version(version: &str) -> bool {
    !version.is_empty()
        && version
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'-' | b'_'))
}

fn write_launchers(
    config: &Config,
    target: &Path,
    jar_name: Option<&str>,
    uses_generated_script: bool,
) -> AppResult<()> {
    let java = config
        .executable_path(&config.minecraft.java_executable)
        .to_string_lossy()
        .into_owned();
    let windows_launcher = if uses_generated_script {
        "@echo off\r\ncall run.bat nogui\r\npause\r\n".to_owned()
    } else {
        let jar_name = jar_name.ok_or_else(|| input_error("Missing Minecraft server jar name"))?;
        format!(
            "@echo off\r\n\"{java}\" -Xms{} -Xmx{} -jar {jar_name} nogui\r\npause\r\n",
            config.minecraft.minimum_memory, config.minecraft.maximum_memory
        )
    };
    fs::write(target.join("start.bat"), windows_launcher)?;
    let shell_path = target.join("start.sh");
    let unix_launcher = if uses_generated_script {
        "#!/bin/sh\nexec ./run.sh nogui\n".to_owned()
    } else {
        let jar_name = jar_name.ok_or_else(|| input_error("Missing Minecraft server jar name"))?;
        format!(
            "#!/bin/sh\nexec \"{java}\" -Xms{} -Xmx{} -jar {jar_name} nogui\n",
            config.minecraft.minimum_memory, config.minecraft.maximum_memory
        )
    };
    fs::write(&shell_path, unix_launcher)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&shell_path, fs::Permissions::from_mode(0o755))?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{is_valid_version, prepare_minecraft_target};
    use crate::minecraft::installer::ServerKind;
    use std::fs;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicUsize, Ordering};

    fn temp_dir() -> PathBuf {
        static NEXT_ID: AtomicUsize = AtomicUsize::new(0);
        std::env::temp_dir().join(format!(
            "gsbuilder-mc-target-{}-{}",
            std::process::id(),
            NEXT_ID.fetch_add(1, Ordering::Relaxed)
        ))
    }

    #[test]
    fn accepts_common_minecraft_versions_and_rejects_url_characters() {
        assert!(is_valid_version("1.21.1"));
        assert!(is_valid_version("25w14a"));
        assert!(!is_valid_version("../1.21"));
        assert!(!is_valid_version(""));
    }

    #[test]
    fn permits_retry_when_fabric_installer_is_the_only_leftover() {
        let target = temp_dir();
        fs::create_dir_all(&target).unwrap();
        fs::write(target.join("fabric-installer.jar"), b"partial download").unwrap();

        assert!(prepare_minecraft_target(&target, ServerKind::Fabric).is_ok());

        fs::remove_dir_all(target).unwrap();
    }

    #[test]
    fn rejects_non_installer_files_and_vanilla_retry() {
        let target = temp_dir();
        fs::create_dir_all(&target).unwrap();
        fs::write(target.join("fabric-installer.jar"), b"partial download").unwrap();

        assert!(prepare_minecraft_target(&target, ServerKind::Vanilla).is_err());
        fs::write(target.join("keep.txt"), b"do not overwrite").unwrap();
        assert!(prepare_minecraft_target(&target, ServerKind::Fabric).is_err());

        fs::remove_dir_all(target).unwrap();
    }
}
