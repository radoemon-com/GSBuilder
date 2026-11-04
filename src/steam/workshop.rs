use crate::core::config::Config;
use crate::core::error::{AppResult, input_error};
use crate::core::language::Language;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

const L4D2_WORKSHOP_APP_ID: &str = "550";
const COLLECTION_BEGIN: &str = "// GSBuilder workshop collection (begin)";
const COLLECTION_END: &str = "// GSBuilder workshop collection (end)";

pub(super) fn install_item(
    config: &Config,
    language: &Language,
    server_path: &Path,
    workshop_id: &str,
) -> AppResult<()> {
    validate_workshop_id(workshop_id)?;
    let game_directory = l4d2_directory(server_path)?;
    let steamcmd = config.executable_path(&config.steam.steamcmd_executable);
    let username = steam_username(config, language)?;
    let mut args = vec![
        "+force_install_dir".to_owned(),
        server_path.to_string_lossy().into_owned(),
        "+login".to_owned(),
        username,
        "+workshop_download_item".to_owned(),
        L4D2_WORKSHOP_APP_ID.to_owned(),
        workshop_id.to_owned(),
    ];
    if config.steam.validate_install {
        args.push("validate".to_owned());
    }
    args.push("+quit".to_owned());
    let status = super::run_steamcmd(&steamcmd, &args, &config.steam.run_as_user)?;
    if !status.success() {
        return Err(input_error(format!(
            "SteamCMD failed to download Workshop item {workshop_id}: {status}"
        )));
    }

    let user_home = steam_user_home(&config.steam.run_as_user)?;
    let content = workshop_content_directory(&steamcmd, &user_home, server_path, workshop_id)
        .ok_or_else(|| {
            input_error(format!(
                "SteamCMD reported success, but Workshop item {workshop_id} was not found in its content directory"
            ))
        })?;
    let installed = install_vpks(&content, &game_directory.join("addons/workshop"))?;
    println!("Installed {installed} Workshop VPK file(s) from item {workshop_id}.");
    Ok(())
}

pub(super) fn configure_collection(server_path: &Path, collection_id: &str) -> AppResult<()> {
    validate_workshop_id(collection_id)?;
    let game_directory = l4d2_directory(server_path)?;
    let autoexec = game_directory.join("cfg/autoexec.cfg");
    let contents = fs::read_to_string(&autoexec).or_else(|error| {
        if error.kind() == std::io::ErrorKind::NotFound {
            Ok(String::new())
        } else {
            Err(error)
        }
    })?;
    let updated = update_collection_setting(&contents, collection_id)?;
    if let Some(parent) = autoexec.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(autoexec, updated)?;
    println!("Configured Workshop collection {collection_id} for server startup.");
    Ok(())
}

fn l4d2_directory(server_path: &Path) -> AppResult<PathBuf> {
    let path = server_path.join("left4dead2");
    if !path.is_dir() {
        return Err(input_error(format!(
            "Not a Left 4 Dead 2 dedicated server directory (missing {}): {}",
            path.display(),
            server_path.display()
        )));
    }
    Ok(path)
}

fn validate_workshop_id(id: &str) -> AppResult<()> {
    if id.is_empty() || !id.bytes().all(|byte| byte.is_ascii_digit()) {
        return Err(input_error(
            "Steam Workshop item and collection IDs must contain only digits",
        ));
    }
    Ok(())
}

fn steam_username(config: &Config, language: &Language) -> AppResult<String> {
    match config.steam.login_mode.as_str() {
        "anonymous" => Ok("anonymous".to_owned()),
        "account" => super::prompt_for_steam_username(language),
        mode => Err(input_error(format!("Unsupported Steam login mode: {mode}"))),
    }
}

fn steam_user_home(run_as_user: &str) -> AppResult<PathBuf> {
    if run_as_user.is_empty() {
        return std::env::var_os("HOME")
            .map(PathBuf::from)
            .ok_or_else(|| input_error("Could not determine the current user's home directory"));
    }
    let output = Command::new("getent")
        .args(["passwd", run_as_user])
        .output()?;
    if !output.status.success() {
        return Err(input_error(format!(
            "Linux user does not exist: {run_as_user}"
        )));
    }
    let entry = String::from_utf8_lossy(&output.stdout);
    let fields = entry.trim().split(':').collect::<Vec<_>>();
    if fields.len() < 6 || fields[0] != run_as_user {
        return Err(input_error(format!(
            "Could not resolve Linux user: {run_as_user}"
        )));
    }
    Ok(PathBuf::from(fields[5]))
}

fn workshop_content_directory(
    steamcmd: &Path,
    home: &Path,
    server_path: &Path,
    workshop_id: &str,
) -> Option<PathBuf> {
    let steamcmd_root = steamcmd.parent()?;
    let relative = Path::new("steamapps")
        .join("workshop/content")
        .join(L4D2_WORKSHOP_APP_ID)
        .join(workshop_id);
    let candidates = [
        steamcmd_root.join(&relative),
        server_path.join(&relative),
        home.join(".local/share/Steam").join(&relative),
        home.join("Steam").join(&relative),
        home.join(".steam/steam").join(&relative),
        home.join(".steam/root").join(&relative),
    ];
    candidates.into_iter().find(|path| path.is_dir())
}

fn install_vpks(source: &Path, destination: &Path) -> AppResult<usize> {
    let mut files = Vec::new();
    collect_vpk_files(source, &mut files)?;
    if files.is_empty() {
        return Err(input_error(format!(
            "Workshop item contains no VPK files: {}",
            source.display()
        )));
    }
    for file in &files {
        let name = file
            .file_name()
            .ok_or_else(|| input_error("Workshop VPK file has no name"))?;
        let target = destination.join(name);
        if target.exists() {
            return Err(input_error(format!(
                "Workshop VPK already exists: {}",
                target.display()
            )));
        }
    }
    fs::create_dir_all(destination)?;
    for file in &files {
        fs::copy(
            file,
            destination.join(
                file.file_name()
                    .ok_or_else(|| input_error("Workshop VPK file has no name"))?,
            ),
        )?;
    }
    Ok(files.len())
}

fn collect_vpk_files(directory: &Path, output: &mut Vec<PathBuf>) -> AppResult<()> {
    for entry in fs::read_dir(directory)? {
        let entry = entry?;
        let path = entry.path();
        let file_type = entry.file_type()?;
        if file_type.is_symlink() {
            return Err(input_error(format!(
                "Symbolic links are not supported in Workshop items: {}",
                path.display()
            )));
        }
        if file_type.is_dir() {
            collect_vpk_files(&path, output)?;
        } else if file_type.is_file()
            && path
                .extension()
                .is_some_and(|extension| extension.eq_ignore_ascii_case("vpk"))
        {
            output.push(path);
        }
    }
    Ok(())
}

fn update_collection_setting(contents: &str, collection_id: &str) -> AppResult<String> {
    let begin = contents.find(COLLECTION_BEGIN);
    let end = contents.find(COLLECTION_END);
    let block =
        format!("{COLLECTION_BEGIN}\nhost_workshop_collection {collection_id}\n{COLLECTION_END}");
    match (begin, end) {
        (Some(begin), Some(end)) if begin < end => {
            let end = end + COLLECTION_END.len();
            let mut updated = String::with_capacity(contents.len() + block.len());
            updated.push_str(&contents[..begin]);
            updated.push_str(&block);
            updated.push_str(&contents[end..]);
            Ok(updated)
        }
        (None, None) => {
            if contents
                .lines()
                .any(|line| line.trim_start().starts_with("host_workshop_collection"))
            {
                return Err(input_error(
                    "autoexec.cfg already sets host_workshop_collection; remove or edit that setting before using GSBuilder",
                ));
            }
            let mut updated = contents.to_owned();
            if !updated.is_empty() && !updated.ends_with('\n') {
                updated.push('\n');
            }
            updated.push_str(&block);
            updated.push('\n');
            Ok(updated)
        }
        _ => Err(input_error(
            "autoexec.cfg contains an incomplete GSBuilder workshop collection block",
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::{install_vpks, update_collection_setting, validate_workshop_id};
    use std::fs;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicUsize, Ordering};

    fn temp_dir() -> PathBuf {
        static NEXT_ID: AtomicUsize = AtomicUsize::new(0);
        std::env::temp_dir().join(format!(
            "gsbuilder-workshop-{}-{}",
            std::process::id(),
            NEXT_ID.fetch_add(1, Ordering::Relaxed)
        ))
    }

    #[test]
    fn validates_workshop_ids_as_digits() {
        assert!(validate_workshop_id("123456").is_ok());
        assert!(validate_workshop_id("").is_err());
        assert!(validate_workshop_id("../123").is_err());
    }

    #[test]
    fn adds_and_updates_managed_collection_setting_without_losing_other_config() {
        let initial = "hostname test\n";
        let configured = update_collection_setting(initial, "123").unwrap();
        assert!(configured.starts_with(initial));
        assert!(configured.contains("host_workshop_collection 123"));
        let updated = update_collection_setting(&configured, "456").unwrap();
        assert!(updated.contains("host_workshop_collection 456"));
        assert!(!updated.contains("host_workshop_collection 123"));
    }

    #[test]
    fn rejects_conflicting_or_incomplete_collection_settings() {
        assert!(update_collection_setting("host_workshop_collection 12\n", "34").is_err());
        assert!(update_collection_setting("// GSBuilder workshop collection (begin)", "34").is_err());
    }

    #[test]
    fn installs_vpk_files_and_refuses_to_overwrite_existing_files() {
        let root = temp_dir();
        let source = root.join("source");
        let destination = root.join("server/addons/workshop");
        fs::create_dir_all(source.join("nested")).unwrap();
        fs::write(source.join("nested/addon.VPK"), b"addon").unwrap();
        fs::write(source.join("readme.txt"), b"ignore").unwrap();
        assert_eq!(install_vpks(&source, &destination).unwrap(), 1);
        assert_eq!(fs::read(destination.join("addon.VPK")).unwrap(), b"addon");
        assert!(install_vpks(&source, &destination).is_err());
        fs::remove_dir_all(root).unwrap();
    }
}
