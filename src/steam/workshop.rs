use crate::core::config::Config;
use crate::core::error::{AppResult, input_error};
use crate::core::http::http_client;
use crate::core::language::Language;
use serde::Deserialize;
use std::collections::HashSet;
use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::Command;

const L4D2_WORKSHOP_APP_ID: &str = "550";
const COLLECTION_BEGIN: &str = "# GSBuilder workshop collection (begin)";
const COLLECTION_END: &str = "# GSBuilder workshop collection (end)";
const COLLECTION_DETAILS_URL: &str =
    "https://api.steampowered.com/ISteamRemoteStorage/GetCollectionDetails/v1/";
const MAX_COLLECTION_ITEMS: usize = 500;
const MAX_COLLECTION_RESPONSE_SIZE: u64 = 2 * 1024 * 1024;

pub(super) fn install_item(
    config: &Config,
    language: &Language,
    server_path: &Path,
    workshop_id: &str,
) -> AppResult<()> {
    validate_workshop_id(workshop_id)?;
    let game_directory = l4d2_directory(server_path)?;
    let username = steam_username(config, language)?;
    let content = download_item(config, server_path, workshop_id, &username)?;
    let installed = install_vpks(&content, &game_directory.join("addons/workshop"))?;
    println!("Installed {installed} Workshop VPK file(s) from item {workshop_id}.");
    Ok(())
}

pub(super) fn install_collection(
    config: &Config,
    language: &Language,
    server_path: &Path,
    collection_id: &str,
) -> AppResult<()> {
    validate_workshop_id(collection_id)?;
    let game_directory = l4d2_directory(server_path)?;
    let items = fetch_collection_items(config, collection_id)?;
    let username = steam_username(config, language)?;
    let mut content_directories = Vec::with_capacity(items.len());
    for (index, item_id) in items.iter().enumerate() {
        println!(
            "Downloading Workshop item {} of {}: {item_id}",
            index + 1,
            items.len()
        );
        content_directories.push(download_item(config, server_path, item_id, &username)?);
    }

    let installed = install_vpk_sources(
        &content_directories,
        &game_directory.join("addons/workshop"),
    )?;
    write_collection_launcher(server_path, collection_id)?;
    println!(
        "Downloaded collection {collection_id}: {} items, {installed} VPK file(s).",
        items.len()
    );
    Ok(())
}

fn write_collection_launcher(server_path: &Path, collection_id: &str) -> AppResult<()> {
    let launcher = server_path.join("start-workshop.sh");
    let contents = fs::read_to_string(&launcher).or_else(|error| {
        if error.kind() == std::io::ErrorKind::NotFound {
            Ok(String::new())
        } else {
            Err(error)
        }
    })?;
    let updated = update_collection_launcher(&contents, collection_id)?;
    fs::write(&launcher, updated)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&launcher, fs::Permissions::from_mode(0o755))?;
    }
    println!(
        "Created {}. Pass normal srcds_run arguments to this launcher.",
        launcher.display()
    );
    Ok(())
}

#[derive(Deserialize)]
struct CollectionApiResponse {
    response: CollectionResponse,
}

#[derive(Deserialize)]
struct CollectionResponse {
    collectiondetails: Vec<CollectionDetails>,
}

#[derive(Deserialize)]
struct CollectionDetails {
    result: u32,
    #[serde(default)]
    children: Vec<CollectionChild>,
}

#[derive(Deserialize)]
struct CollectionChild {
    publishedfileid: String,
}

fn fetch_collection_items(config: &Config, collection_id: &str) -> AppResult<Vec<String>> {
    validate_workshop_id(collection_id)?;
    let response = http_client(config)?
        .post(COLLECTION_DETAILS_URL)
        .form(&[
            ("collectioncount", "1"),
            ("publishedfileids[0]", collection_id),
        ])
        .send()?
        .error_for_status()?;
    if response
        .content_length()
        .is_some_and(|length| length > MAX_COLLECTION_RESPONSE_SIZE)
    {
        return Err(input_error(
            "Steam Workshop collection response exceeds the 2 MiB size limit",
        ));
    }
    let mut body = Vec::new();
    response
        .take(MAX_COLLECTION_RESPONSE_SIZE + 1)
        .read_to_end(&mut body)?;
    if body.len() as u64 > MAX_COLLECTION_RESPONSE_SIZE {
        return Err(input_error(
            "Steam Workshop collection response exceeds the 2 MiB size limit",
        ));
    }
    parse_collection_items(&body).map_err(|error| {
        input_error(format!(
            "Could not read Workshop collection {collection_id}: {error}"
        ))
    })
}

fn download_item(
    config: &Config,
    server_path: &Path,
    workshop_id: &str,
    username: &str,
) -> AppResult<PathBuf> {
    let steamcmd = config.executable_path(&config.steam.steamcmd_executable);
    let mut args = vec![
        "+force_install_dir".to_owned(),
        server_path.to_string_lossy().into_owned(),
        "+login".to_owned(),
        username.to_owned(),
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
    workshop_content_directory(&steamcmd, &user_home, server_path, workshop_id).ok_or_else(|| {
        input_error(format!(
            "SteamCMD reported success, but Workshop item {workshop_id} was not found in its content directory"
        ))
    })
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
    install_vpk_sources(&[source.to_path_buf()], destination)
}

fn install_vpk_sources(sources: &[PathBuf], destination: &Path) -> AppResult<usize> {
    let mut files = Vec::new();
    for source in sources {
        collect_vpk_files(source, &mut files)?;
    }
    if files.is_empty() {
        return Err(input_error(format!(
            "Workshop download contains no VPK files in {} item(s)",
            sources.len()
        )));
    }
    let mut names = HashSet::new();
    for file in &files {
        let name = file
            .file_name()
            .ok_or_else(|| input_error("Workshop VPK file has no name"))?;
        if !names.insert(name.to_os_string()) {
            return Err(input_error(format!(
                "Workshop items contain VPK files with the same name: {}",
                name.to_string_lossy()
            )));
        }
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

fn parse_collection_items(body: &[u8]) -> AppResult<Vec<String>> {
    let response: CollectionApiResponse = serde_json::from_slice(body)?;
    if response.response.collectiondetails.len() != 1 {
        return Err(input_error(
            "Steam did not return exactly one Workshop collection",
        ));
    }
    let collection = &response.response.collectiondetails[0];
    if collection.result != 1 {
        return Err(input_error(format!(
            "Steam could not load the Workshop collection (result {})",
            collection.result
        )));
    }
    if collection.children.is_empty() {
        return Err(input_error("Workshop collection is empty"));
    }
    if collection.children.len() > MAX_COLLECTION_ITEMS {
        return Err(input_error(format!(
            "Workshop collection has more than the supported {MAX_COLLECTION_ITEMS} items"
        )));
    }
    let mut seen = HashSet::new();
    let mut items = Vec::with_capacity(collection.children.len());
    for child in &collection.children {
        validate_workshop_id(&child.publishedfileid)?;
        if seen.insert(child.publishedfileid.as_str()) {
            items.push(child.publishedfileid.clone());
        }
    }
    Ok(items)
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

fn update_collection_launcher(contents: &str, collection_id: &str) -> AppResult<String> {
    let begin = contents.find(COLLECTION_BEGIN);
    let end = contents.find(COLLECTION_END);
    let block = format!(
        "{COLLECTION_BEGIN}\nexec \"$launcher\" -game left4dead2 \"$@\" +host_workshop_collection {collection_id}\n{COLLECTION_END}"
    );
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
            if !contents.is_empty() {
                return Err(input_error(
                    "start-workshop.sh already exists and is not managed by GSBuilder; move it aside before configuring a Workshop collection",
                ));
            }
            Ok(format!(
                "#!/bin/sh\nset -eu\n\nserver_root=$(CDPATH= cd -- \"$(dirname -- \"$0\")\" && pwd)\nif [ -x \"$server_root/srcds_run\" ]; then\n    launcher=\"$server_root/srcds_run\"\nelif [ -x \"$server_root/srcds_run.sh\" ]; then\n    launcher=\"$server_root/srcds_run.sh\"\nelse\n    printf '%s\\n' \"Could not find srcds_run in $server_root\" >&2\n    exit 1\nfi\n\n{block}\n"
            ))
        }
        _ => Err(input_error(
            "start-workshop.sh contains an incomplete GSBuilder collection block",
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::{
        COLLECTION_BEGIN, install_vpks, parse_collection_items, update_collection_launcher,
        validate_workshop_id, write_collection_launcher,
    };
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
    fn parses_collection_items_and_deduplicates_ids() {
        let body = br#"{"response":{"collectiondetails":[{"result":1,"children":[{"publishedfileid":"123"},{"publishedfileid":"456"},{"publishedfileid":"123"}]}]}}"#;
        assert_eq!(parse_collection_items(body).unwrap(), ["123", "456"]);
    }

    #[test]
    fn rejects_failed_empty_and_invalid_workshop_collections() {
        assert!(
            parse_collection_items(
                br#"{"response":{"collectiondetails":[{"result":2,"children":[]}]}}"#
            )
            .is_err()
        );
        assert!(
            parse_collection_items(
                br#"{"response":{"collectiondetails":[{"result":1,"children":[]}]}}"#
            )
            .is_err()
        );
        assert!(
            parse_collection_items(
                br#"{"response":{"collectiondetails":[{"result":1,"children":[{"publishedfileid":"../x"}]}]}}"#
            )
            .is_err()
        );
    }

    #[test]
    fn generates_and_updates_a_collection_startup_script() {
        let configured = update_collection_launcher("", "123").unwrap();
        assert!(configured.starts_with("#!/bin/sh\n"));
        assert!(
            configured.contains(
                "exec \"$launcher\" -game left4dead2 \"$@\" +host_workshop_collection 123"
            )
        );
        let updated = update_collection_launcher(&configured, "456").unwrap();
        assert!(updated.contains("+host_workshop_collection 456"));
        assert!(!updated.contains("+host_workshop_collection 123"));
    }

    #[cfg(unix)]
    #[test]
    fn generated_launcher_passes_collection_and_server_arguments_to_srcds() {
        use std::os::unix::fs::PermissionsExt;

        let root = temp_dir();
        let game_directory = root.join("left4dead2");
        fs::create_dir_all(&game_directory).unwrap();
        let srcds = root.join("srcds_run");
        fs::write(
            &srcds,
            "#!/bin/sh\nprintf '%s\\n' \"$@\" > \"$GSBUILDER_TEST_ARGS\"\n",
        )
        .unwrap();
        fs::set_permissions(&srcds, fs::Permissions::from_mode(0o755)).unwrap();

        write_collection_launcher(&root, "123456").unwrap();
        let launcher = root.join("start-workshop.sh");
        assert!(fs::metadata(&launcher).unwrap().permissions().mode() & 0o111 != 0);
        let args_file = root.join("args.txt");
        let status = std::process::Command::new(&launcher)
            .args(["-console", "+map", "c1m1_hotel"])
            .env("GSBUILDER_TEST_ARGS", &args_file)
            .status()
            .unwrap();
        assert!(status.success());
        let args = fs::read_to_string(args_file).unwrap();
        assert!(args.contains("-game\nleft4dead2\n"));
        assert!(args.contains("-console\n+map\nc1m1_hotel\n"));
        assert!(args.contains("+host_workshop_collection\n123456\n"));
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn rejects_conflicting_or_incomplete_collection_settings() {
        assert!(update_collection_launcher("# custom launcher\n", "34").is_err());
        assert!(update_collection_launcher(COLLECTION_BEGIN, "34").is_err());
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
