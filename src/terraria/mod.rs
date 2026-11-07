use crate::core::config::Config;
use crate::core::error::{AppResult, input_error};
use crate::core::language::Language;
use crate::core::paths::{is_valid_component, prepare_target};
use crate::core::servers::remove_server;
use crate::steam::{prompt_for_steam_username, run_steamcmd};
use serde_json::Value;
use std::fs;
use std::path::{Path, PathBuf};

const TERRARIA_APP_ID: &str = "105600";
const TMODLOADER_APP_ID: &str = "1281930";
const TMODLOADER_WORKSHOP_APP_ID: &str = "1281930";
const SERVER_KIND_FILE: &str = ".gsbuilder-terraria-server";

#[derive(Clone, Copy, PartialEq, Eq)]
enum ServerKind {
    Vanilla,
    TmodLoader,
}

fn marker_matches_kind(target: &Path, kind: ServerKind) -> bool {
    let marker = target.join(SERVER_KIND_FILE);
    fs::symlink_metadata(&marker).is_ok_and(|metadata| {
        metadata.is_file()
            && fs::read_to_string(marker)
                .is_ok_and(|installed_kind| installed_kind.trim() == kind.as_str())
    })
}

impl ServerKind {
    fn parse(value: &str) -> AppResult<Self> {
        match value {
            "vanilla" => Ok(Self::Vanilla),
            "tmodloader" => Ok(Self::TmodLoader),
            _ => Err(input_error("Terraria type must be vanilla or tmodloader")),
        }
    }

    fn as_str(self) -> &'static str {
        match self {
            Self::Vanilla => "vanilla",
            Self::TmodLoader => "tmodloader",
        }
    }

    fn app_id(self) -> &'static str {
        match self {
            Self::Vanilla => TERRARIA_APP_ID,
            Self::TmodLoader => TMODLOADER_APP_ID,
        }
    }
}

pub struct TerrariaService<'a> {
    config: &'a Config,
    language: &'a Language,
}

impl<'a> TerrariaService<'a> {
    pub fn new(config: &'a Config, language: &'a Language) -> Self {
        Self { config, language }
    }

    pub fn create(&self, kind: &str, server_name: &str) -> AppResult<()> {
        if !is_valid_component(server_name) {
            return Err(input_error(self.language.text("paths.component.invalid")?));
        }
        let kind = ServerKind::parse(kind)?;
        let target = self.config.terraria_servers_path().join(server_name);
        prepare_terraria_target(&target, kind)?;
        fs::write(target.join(SERVER_KIND_FILE), kind.as_str())?;
        let install_directory = server_install_directory(&target, kind);
        fs::create_dir_all(&install_directory)?;

        let username = match self.config.steam.login_mode.as_str() {
            "anonymous" => "anonymous".to_owned(),
            "account" => prompt_for_steam_username(self.language)?,
            mode => return Err(input_error(format!("Unsupported Steam login mode: {mode}"))),
        };
        let steamcmd = self
            .config
            .executable_path(&self.config.steam.steamcmd_executable);
        let mut args = vec![
            "+force_install_dir".to_owned(),
            install_directory.to_string_lossy().into_owned(),
            "+login".to_owned(),
            username,
            "+app_update".to_owned(),
            kind.app_id().to_owned(),
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
        if !server_files_exist(&target, kind) {
            return Err(input_error(format!(
                "SteamCMD completed, but the {} server files were not found in {}",
                kind.as_str(),
                target.display()
            )));
        }

        fs::create_dir_all(target.join("Worlds"))?;
        write_server_config(&target)?;
        write_launchers(&target, kind)?;
        println!(
            "{} {}",
            self.language.text("server.ready")?,
            target.display()
        );
        Ok(())
    }

    pub fn remove_server(&self, server_name: &str) -> AppResult<()> {
        if !is_valid_component(server_name) {
            return Err(input_error(self.language.text("paths.component.invalid")?));
        }
        let path = remove_server(&self.config.terraria_servers_path(), server_name)?;
        println!(
            "{}",
            self.language
                .format("server.deleted", &[("path", &path.display().to_string())])?
        );
        Ok(())
    }

    pub fn add_workshop_mod(
        &self,
        server_name: &str,
        workshop_id: &str,
        mod_name: &str,
    ) -> AppResult<()> {
        validate_workshop_id(workshop_id)?;
        validate_mod_name(mod_name)?;
        let server = self.tmodloader_server_path(server_name)?;
        let (mut workshop_ids, mut enabled_mods) = read_mod_lists(&server)?;

        let steamcmd = self
            .config
            .executable_path(&self.config.steam.steamcmd_executable);
        let args = vec![
            "+force_install_dir".to_owned(),
            server.to_string_lossy().into_owned(),
            "+login".to_owned(),
            "anonymous".to_owned(),
            "+workshop_download_item".to_owned(),
            TMODLOADER_WORKSHOP_APP_ID.to_owned(),
            workshop_id.to_owned(),
            "+quit".to_owned(),
        ];
        let status = run_steamcmd(&steamcmd, &args, &self.config.steam.run_as_user)?;
        if !status.success() {
            return Err(input_error(format!(
                "SteamCMD failed to download tModLoader Workshop item {workshop_id}: {status}"
            )));
        }

        let workshop_item = server
            .join("steamapps/workshop/content")
            .join(TMODLOADER_WORKSHOP_APP_ID)
            .join(workshop_id);
        if !workshop_item.is_dir() {
            return Err(input_error(format!(
                "SteamCMD completed, but tModLoader Workshop item {workshop_id} was not found in {}",
                workshop_item.display()
            )));
        }

        append_unique(&mut workshop_ids, workshop_id);
        append_unique(&mut enabled_mods, mod_name);
        write_mod_lists(&server, &workshop_ids, &enabled_mods)?;
        println!(
            "{}",
            self.language.format(
                "terraria.mod.added",
                &[("name", mod_name), ("path", &server.display().to_string())],
            )?
        );
        Ok(())
    }

    pub fn add_mod_file(&self, server_name: &str, source: &Path) -> AppResult<()> {
        let server = self.tmodloader_server_path(server_name)?;
        let mod_name = local_mod_name(source)?;

        let (workshop_ids, mut enabled_mods) = read_mod_lists(&server)?;
        let mods_directory = server.join("Mods");
        fs::create_dir_all(&mods_directory)?;
        let destination = mods_directory.join(
            source
                .file_name()
                .ok_or_else(|| input_error("The .tmod file has no file name"))?,
        );
        if destination.exists() {
            return Err(input_error(format!(
                "Mod file already exists: {}",
                destination.display()
            )));
        }
        append_unique(&mut enabled_mods, &mod_name);
        fs::copy(source, &destination)?;
        if let Err(error) = write_mod_lists(&server, &workshop_ids, &enabled_mods) {
            fs::remove_file(&destination)?;
            return Err(error);
        }
        println!(
            "{}",
            self.language.format(
                "terraria.mod.added",
                &[("name", &mod_name), ("path", &server.display().to_string())],
            )?
        );
        Ok(())
    }

    fn tmodloader_server_path(&self, server_name: &str) -> AppResult<PathBuf> {
        if !is_valid_component(server_name) {
            return Err(input_error(self.language.text("paths.component.invalid")?));
        }
        let server = self.config.terraria_servers_path().join(server_name);
        if !server.is_dir() {
            return Err(input_error(format!(
                "Terraria server does not exist: {}",
                server.display()
            )));
        }
        if !marker_matches_kind(&server, ServerKind::TmodLoader) {
            return Err(input_error(format!(
                "Mods can only be installed on a GSBuilder tModLoader server: {}",
                server.display()
            )));
        }
        Ok(server)
    }
}

fn prepare_terraria_target(target: &Path, kind: ServerKind) -> AppResult<()> {
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
    if marker_matches_kind(target, kind) {
        return Ok(());
    }
    Err(input_error(format!(
        "Install directory must be empty or belong to the same GSBuilder Terraria server type: {}",
        target.display()
    )))
}

fn server_files_exist(target: &Path, kind: ServerKind) -> bool {
    match kind {
        ServerKind::Vanilla => [
            "TerrariaServer.bin.x86_64",
            "TerrariaServer.exe",
            "Linux/TerrariaServer.bin.x86_64",
        ]
        .iter()
        .any(|path| target.join(path).is_file()),
        ServerKind::TmodLoader => [
            "server/LaunchUtils/ScriptCaller.sh",
            "server/start-tModLoaderServer.sh",
            "server/start-tModLoaderServer.bat",
            "server/tModLoaderServer",
            "server/tModLoaderServer.exe",
        ]
        .iter()
        .any(|path| target.join(path).is_file()),
    }
}

fn write_server_config(target: &Path) -> AppResult<()> {
    let path = target.join("serverconfig.txt");
    if !path.exists() {
        fs::write(
            path,
            "world=./Worlds/world.wld\nautocreate=1\nmaxplayers=8\nport=7777\nsecure=1\n",
        )?;
    }
    Ok(())
}

fn write_launchers(target: &Path, kind: ServerKind) -> AppResult<()> {
    let shell_launcher = match kind {
        ServerKind::Vanilla => {
            "#!/bin/sh\ncd \"$(dirname -- \"$0\")\"\nif [ -x ./TerrariaServer.bin.x86_64 ]; then exec ./TerrariaServer.bin.x86_64 -config serverconfig.txt \"$@\"; fi\nif [ -x ./Linux/TerrariaServer.bin.x86_64 ]; then exec ./Linux/TerrariaServer.bin.x86_64 -config serverconfig.txt \"$@\"; fi\nprintf '%s\\n' 'Could not find TerrariaServer.bin.x86_64.' >&2\nexit 1\n"
        }
        ServerKind::TmodLoader => {
            "#!/bin/sh\ncd \"$(dirname -- \"$0\")\"\nif [ -f ./server/LaunchUtils/ScriptCaller.sh ]; then cd ./server || exit 1; exec bash -c 'source ./LaunchUtils/ScriptCaller.sh -server -config ../serverconfig.txt -steamworkshopfolder \"$PWD/../steamapps/workshop\" -tmlsavedirectory \"$PWD/..\" \"$@\"' gsbuilder \"$@\"; fi\nif [ -f ./server/start-tModLoaderServer.sh ]; then cd ./server || exit 1; exec sh ./start-tModLoaderServer.sh -config ../serverconfig.txt -steamworkshopfolder \"$PWD/../steamapps/workshop\" -tmlsavedirectory \"$PWD/..\" \"$@\"; fi\nif [ -x ./server/tModLoaderServer ]; then cd ./server || exit 1; exec ./tModLoaderServer -config ../serverconfig.txt \"$@\"; fi\nprintf '%s\\n' 'Could not find a tModLoader server launcher in ./server.' >&2\nexit 1\n"
        }
    };
    let windows_launcher = match kind {
        ServerKind::Vanilla => {
            "@echo off\r\ncd /d \"%~dp0\"\r\nTerrariaServer.exe -config serverconfig.txt %*\r\n"
        }
        ServerKind::TmodLoader => {
            "@echo off\r\ncd /d \"%~dp0server\"\r\nif exist start-tModLoaderServer.bat (call start-tModLoaderServer.bat -config ..\\serverconfig.txt -steamworkshopfolder ..\\steamapps\\workshop -tmlsavedirectory .. %* & exit /b %errorlevel%)\r\nif exist tModLoaderServer.exe (tModLoaderServer.exe -config ..\\serverconfig.txt %* & exit /b %errorlevel%)\r\necho Could not find the tModLoader server launcher in .\\server. 1>&2\r\nexit /b 1\r\n"
        }
    };
    let shell_path = target.join("start.sh");
    fs::write(&shell_path, shell_launcher)?;
    fs::write(target.join("start.bat"), windows_launcher)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&shell_path, fs::Permissions::from_mode(0o755))?;
    }
    Ok(())
}

fn server_install_directory(target: &Path, kind: ServerKind) -> PathBuf {
    match kind {
        ServerKind::Vanilla => target.to_path_buf(),
        ServerKind::TmodLoader => target.join("server"),
    }
}

fn validate_workshop_id(id: &str) -> AppResult<()> {
    if id.is_empty() || !id.bytes().all(|byte| byte.is_ascii_digit()) {
        return Err(input_error(
            "tModLoader Workshop IDs must contain digits only",
        ));
    }
    Ok(())
}

fn validate_mod_name(name: &str) -> AppResult<()> {
    if name.is_empty()
        || !name
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_')
    {
        return Err(input_error(
            "tModLoader mod names must contain only ASCII letters, digits, or underscores",
        ));
    }
    Ok(())
}

fn local_mod_name(source: &Path) -> AppResult<String> {
    let metadata = fs::symlink_metadata(source)?;
    if !metadata.is_file()
        || source
            .extension()
            .and_then(|extension| extension.to_str())
            .is_none_or(|extension| !extension.eq_ignore_ascii_case("tmod"))
    {
        return Err(input_error(
            "The local tModLoader mod must be a regular .tmod file",
        ));
    }
    let name = source
        .file_stem()
        .and_then(|name| name.to_str())
        .ok_or_else(|| input_error("The .tmod file has no valid mod name"))?;
    validate_mod_name(name)?;
    Ok(name.to_owned())
}

fn read_mod_lists(server: &Path) -> AppResult<(Vec<String>, Vec<String>)> {
    let mods_directory = server.join("Mods");
    let install_path = mods_directory.join("install.txt");
    let workshop_ids = if install_path.exists() {
        fs::read_to_string(install_path)?
            .lines()
            .map(str::trim)
            .filter(|id| !id.is_empty())
            .map(|id| {
                validate_workshop_id(id)?;
                Ok(id.to_owned())
            })
            .collect::<AppResult<Vec<_>>>()?
    } else {
        Vec::new()
    };
    let enabled_path = mods_directory.join("enabled.json");
    let enabled_mods = if enabled_path.exists() {
        let contents = fs::read(enabled_path)?;
        let value: Value = serde_json::from_slice(&contents)?;
        value
            .as_array()
            .ok_or_else(|| input_error("Mods/enabled.json must contain a JSON array"))?
            .iter()
            .map(|name| {
                let name = name
                    .as_str()
                    .ok_or_else(|| input_error("Mods/enabled.json must contain mod names"))?;
                validate_mod_name(name)?;
                Ok(name.to_owned())
            })
            .collect::<AppResult<Vec<_>>>()?
    } else {
        Vec::new()
    };
    Ok((workshop_ids, enabled_mods))
}

fn write_mod_lists(
    server: &Path,
    workshop_ids: &[String],
    enabled_mods: &[String],
) -> AppResult<()> {
    let mods_directory = server.join("Mods");
    fs::create_dir_all(&mods_directory)?;
    let install_content = if workshop_ids.is_empty() {
        String::new()
    } else {
        format!("{}\n", workshop_ids.join("\n"))
    };
    fs::write(mods_directory.join("install.txt"), install_content)?;
    let enabled_json = serde_json::to_string_pretty(enabled_mods)?;
    fs::write(mods_directory.join("enabled.json"), enabled_json)?;
    Ok(())
}

fn append_unique(items: &mut Vec<String>, item: &str) {
    if !items.iter().any(|existing| existing == item) {
        items.push(item.to_owned());
    }
}

#[cfg(test)]
mod tests {
    use super::{
        ServerKind, append_unique, local_mod_name, prepare_terraria_target, read_mod_lists,
        server_install_directory, validate_mod_name, validate_workshop_id, write_launchers,
        write_mod_lists, write_server_config,
    };
    use std::fs;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicUsize, Ordering};

    fn temp_dir() -> PathBuf {
        static NEXT_ID: AtomicUsize = AtomicUsize::new(0);
        std::env::temp_dir().join(format!(
            "gsbuilder-terraria-{}-{}",
            std::process::id(),
            NEXT_ID.fetch_add(1, Ordering::Relaxed)
        ))
    }

    #[test]
    fn validates_workshop_ids_and_tmodloader_names() {
        assert!(validate_workshop_id("123456789").is_ok());
        assert!(validate_workshop_id("").is_err());
        assert!(validate_workshop_id("123;quit").is_err());
        assert!(validate_mod_name("CalamityMod").is_ok());
        assert!(validate_mod_name("Mod_Name2").is_ok());
        assert!(validate_mod_name("../mod").is_err());
        assert!(validate_mod_name("mod name").is_err());
    }

    #[test]
    fn mod_lists_preserve_existing_workshop_and_enabled_entries() {
        let server = temp_dir();
        fs::create_dir_all(&server).unwrap();
        write_mod_lists(&server, &["123".to_owned()], &["ExistingMod".to_owned()]).unwrap();
        let (mut ids, mut mods) = read_mod_lists(&server).unwrap();
        append_unique(&mut ids, "456");
        append_unique(&mut ids, "123");
        append_unique(&mut mods, "NewMod");
        append_unique(&mut mods, "ExistingMod");
        write_mod_lists(&server, &ids, &mods).unwrap();

        assert_eq!(
            read_mod_lists(&server).unwrap(),
            (
                vec!["123".to_owned(), "456".to_owned()],
                vec!["ExistingMod".to_owned(), "NewMod".to_owned()]
            )
        );
        fs::remove_dir_all(server).unwrap();
    }

    #[test]
    fn target_retry_requires_the_original_server_kind_marker() {
        let target = temp_dir();
        fs::create_dir_all(&target).unwrap();
        fs::write(target.join(".gsbuilder-terraria-server"), "tmodloader").unwrap();

        assert!(prepare_terraria_target(&target, ServerKind::TmodLoader).is_ok());
        assert!(prepare_terraria_target(&target, ServerKind::Vanilla).is_err());

        fs::remove_dir_all(target).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn refuses_a_symlinked_server_kind_marker() {
        let target = temp_dir();
        fs::create_dir_all(&target).unwrap();
        let external_marker = temp_dir().with_extension("outside");
        fs::write(&external_marker, "tmodloader").unwrap();
        std::os::unix::fs::symlink(&external_marker, target.join(".gsbuilder-terraria-server"))
            .unwrap();

        assert!(prepare_terraria_target(&target, ServerKind::TmodLoader).is_err());
        assert_eq!(fs::read_to_string(&external_marker).unwrap(), "tmodloader");

        fs::remove_dir_all(target).unwrap();
        fs::remove_file(external_marker).unwrap();
    }

    #[test]
    fn rejects_invalid_existing_mod_metadata_instead_of_overwriting_it() {
        let server = temp_dir();
        fs::create_dir_all(server.join("Mods")).unwrap();
        fs::write(
            server.join("Mods/enabled.json"),
            r#"{"unexpected":"object"}"#,
        )
        .unwrap();
        assert!(read_mod_lists(&server).is_err());
        fs::remove_dir_all(server).unwrap();
    }

    #[test]
    fn uses_tmodloader_and_vanilla_app_ids() {
        assert_eq!(ServerKind::parse("vanilla").unwrap().app_id(), "105600");
        assert_eq!(ServerKind::parse("tmodloader").unwrap().app_id(), "1281930");
        assert!(ServerKind::parse("modded").is_err());
    }

    #[test]
    fn installs_tmodloader_under_the_official_server_subdirectory() {
        let root = PathBuf::from("servers/Terraria/test");
        assert_eq!(server_install_directory(&root, ServerKind::Vanilla), root);
        assert_eq!(
            server_install_directory(&root, ServerKind::TmodLoader),
            PathBuf::from("servers/Terraria/test/server")
        );
    }

    #[test]
    fn rejects_symlinked_local_mod_files() {
        #[cfg(unix)]
        {
            let root = temp_dir();
            fs::create_dir_all(&root).unwrap();
            let target = root.join("mod.tmod");
            let link = root.join("link.tmod");
            fs::write(&target, b"mod").unwrap();
            std::os::unix::fs::symlink(&target, &link).unwrap();
            assert!(local_mod_name(&link).is_err());
            fs::remove_dir_all(root).unwrap();
        }
    }

    #[test]
    fn validates_local_mod_file_extension_and_name() {
        let root = temp_dir();
        fs::create_dir_all(&root).unwrap();
        let mod_file = root.join("ExampleMod.tmod");
        let wrong_extension = root.join("ExampleMod.zip");
        fs::write(&mod_file, b"mod").unwrap();
        fs::write(&wrong_extension, b"mod").unwrap();

        assert_eq!(local_mod_name(&mod_file).unwrap(), "ExampleMod");
        assert!(local_mod_name(&wrong_extension).is_err());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn writes_default_server_config_without_overwriting_custom_settings() {
        let root = temp_dir();
        fs::create_dir_all(&root).unwrap();
        write_server_config(&root).unwrap();
        let config_path = root.join("serverconfig.txt");
        let defaults = fs::read_to_string(&config_path).unwrap();
        assert!(defaults.contains("autocreate=1"));

        fs::write(&config_path, "maxplayers=16\n").unwrap();
        write_server_config(&root).unwrap();
        assert_eq!(fs::read_to_string(config_path).unwrap(), "maxplayers=16\n");
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn writes_platform_launchers_for_each_server_kind() {
        let vanilla = temp_dir();
        fs::create_dir_all(&vanilla).unwrap();
        write_launchers(&vanilla, ServerKind::Vanilla).unwrap();
        assert!(
            fs::read_to_string(vanilla.join("start.sh"))
                .unwrap()
                .contains("TerrariaServer.bin.x86_64")
        );

        let modded = temp_dir().with_extension("tml");
        fs::create_dir_all(&modded).unwrap();
        write_launchers(&modded, ServerKind::TmodLoader).unwrap();
        assert!(
            fs::read_to_string(modded.join("start.sh"))
                .unwrap()
                .contains("./server/LaunchUtils/ScriptCaller.sh")
        );
        fs::remove_dir_all(vanilla).unwrap();
        fs::remove_dir_all(modded).unwrap();
    }
}
