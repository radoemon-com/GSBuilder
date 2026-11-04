use crate::core::config::Config;
use crate::core::error::{AppResult, input_error};
use crate::core::http::http_client;
use crate::core::language::Language;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet, VecDeque};
use std::fs;
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::process::Command;

const L4D2_WORKSHOP_APP_ID: &str = "550";
const COLLECTION_BEGIN: &str = "# GSBuilder workshop collection (begin)";
const COLLECTION_END: &str = "# GSBuilder workshop collection (end)";
const COLLECTION_DETAILS_URL: &str =
    "https://api.steampowered.com/ISteamRemoteStorage/GetCollectionDetails/v1/";
const PUBLISHED_FILE_DETAILS_URL: &str =
    "https://api.steampowered.com/ISteamRemoteStorage/GetPublishedFileDetails/v1/";
const MAX_COLLECTION_ITEMS: usize = 500;
const MAX_COLLECTIONS: usize = 500;
const API_BATCH_SIZE: usize = 100;
const MAX_COLLECTION_RESPONSE_SIZE: u64 = 2 * 1024 * 1024;

pub(super) fn install_item(
    config: &Config,
    language: &Language,
    server_path: &Path,
    workshop_id: &str,
) -> AppResult<()> {
    let references = parse_workshop_references(workshop_id, false)?;
    let item_ids = resolve_workshop_references(config, references)?;
    let game_directory = l4d2_directory(server_path)?;
    let username = steam_username(config, language)?;
    println!("Workshop items to install: {}", item_ids.join(", "));
    let downloads = download_items(config, server_path, &item_ids, &username)?;
    let manifest = read_manifest(server_path)?;
    let protected_names = manifest
        .plugins
        .values()
        .flat_map(|plugin| plugin.files.iter().cloned())
        .collect::<HashSet<_>>();
    let installed = install_direct_items(
        &item_ids,
        &downloads,
        &game_directory.join("addons/workshop"),
        &protected_names,
        |key, item_id| confirm_for_item(language, key, item_id),
    )?;
    println!("Installed {installed} Workshop VPK file(s).");
    Ok(())
}

fn vpk_output_filename(path: &Path) -> AppResult<String> {
    let name = path
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| input_error("Workshop VPK file has no valid UTF-8 name"))?;
    if is_vpk_filename(name) {
        return Ok(name.to_owned());
    }

    let lower_name = name.to_ascii_lowercase();
    if let Some(stem) = lower_name
        .ends_with("_legacy.bin")
        .then(|| &name[..name.len() - "_legacy.bin".len()])
        .filter(|stem| !stem.is_empty())
    {
        return Ok(format!("{stem}.vpk"));
    }

    Err(input_error(format!(
        "Workshop item contains an invalid VPK filename: {name}"
    )))
}

pub(super) fn install_collection(
    config: &Config,
    language: &Language,
    server_path: &Path,
    collection_id: &str,
) -> AppResult<()> {
    let references = parse_workshop_references(collection_id, true)?;
    let collection_ids = references
        .iter()
        .map(|reference| reference.id.clone())
        .collect::<Vec<_>>();
    let game_directory = l4d2_directory(server_path)?;
    let mut manifest = read_manifest(server_path)?;
    for collection_id in &collection_ids {
        if !manifest.collections.contains(collection_id) {
            manifest.collections.push(collection_id.clone());
        }
    }

    let items = fetch_collection_items(config, &manifest.collections)?;
    let details = fetch_published_file_details(config, &items)?;
    let username = steam_username(config, language)?;
    let destination = game_directory.join("addons/workshop");
    let mut updates = Vec::new();
    for (index, item) in details.iter().enumerate() {
        if plugin_is_current(
            manifest.plugins.get(&item.publishedfileid),
            item.time_updated,
            &destination,
        ) {
            println!(
                "Workshop item {} of {} is up to date: {}",
                index + 1,
                details.len(),
                item.publishedfileid
            );
            continue;
        }
        println!(
            "Downloading Workshop item {} of {}: {item_id}",
            index + 1,
            details.len(),
            item_id = item.publishedfileid
        );
        updates.push(item);
    }

    let update_ids = updates
        .iter()
        .map(|item| item.publishedfileid.clone())
        .collect::<Vec<_>>();
    let downloads = download_items(config, server_path, &update_ids, &username)?;
    let update_sources = updates
        .iter()
        .map(|item| {
            let source = downloads.get(&item.publishedfileid).ok_or_else(|| {
                input_error(format!(
                    "SteamCMD did not provide Workshop item {}",
                    item.publishedfileid
                ))
            })?;
            Ok((*item, source.clone()))
        })
        .collect::<AppResult<Vec<_>>>()?;

    let installed_files = install_vpk_updates(
        &update_sources,
        &mut manifest,
        &destination,
        |key, item_id| confirm_for_item(language, key, item_id),
    )?;
    write_manifest(server_path, &manifest)?;
    let collection_id = collection_ids
        .last()
        .ok_or_else(|| input_error("At least one Workshop collection is required"))?;
    write_collection_launcher(server_path, collection_id)?;
    println!(
        "Synchronized {} registered collection(s), including {}: {} items checked, {} update(s) attempted, {installed_files} VPK file(s) installed.",
        manifest.collections.len(),
        collection_ids.join(", "),
        details.len(),
        updates.len()
    );
    Ok(())
}

pub(super) fn remove_item(
    language: &Language,
    server_path: &Path,
    workshop_input: &str,
) -> AppResult<()> {
    let references = parse_workshop_references(workshop_input, false)?;
    let game_directory = l4d2_directory(server_path)?;
    let destination = game_directory.join("addons/workshop");
    let mut manifest = read_manifest(server_path)?;
    let mut removals = Vec::new();
    for reference in references {
        let mut files = manifest
            .plugins
            .get(&reference.id)
            .map(|plugin| plugin.files.iter().cloned().collect::<HashSet<_>>())
            .unwrap_or_default();
        if destination.is_dir() {
            for entry in fs::read_dir(&destination)? {
                let entry = entry?;
                let Some(name) = entry.file_name().to_str().map(str::to_owned) else {
                    continue;
                };
                if entry.file_type()?.is_file() && is_direct_item_filename(&name, &reference.id) {
                    files.insert(name);
                }
            }
        }
        removals.push((reference.id, files));
    }

    let file_count = removals.iter().map(|(_, files)| files.len()).sum::<usize>();
    if file_count == 0
        && removals
            .iter()
            .all(|(id, _)| !manifest.plugins.contains_key(id))
    {
        println!("{}", language.text("steam.workshop.remove.nothing")?);
        return Ok(());
    }
    if !confirm(language, "steam.workshop.remove.confirm")? {
        println!("{}", language.text("steam.workshop.remove.cancelled")?);
        return Ok(());
    }

    for (item_id, files) in removals {
        for name in files {
            remove_regular_file(&destination.join(name))?;
        }
        manifest.plugins.remove(&item_id);
    }
    write_manifest(server_path, &manifest)?;
    println!("{}", language.text("steam.workshop.remove.done")?);
    Ok(())
}

pub(super) fn remove_collection(
    config: &Config,
    language: &Language,
    server_path: &Path,
    collection_input: &str,
) -> AppResult<()> {
    let references = parse_workshop_references(collection_input, true)?;
    let removed_ids = references
        .iter()
        .map(|reference| reference.id.clone())
        .collect::<HashSet<_>>();
    let game_directory = l4d2_directory(server_path)?;
    let destination = game_directory.join("addons/workshop");
    let mut manifest = read_manifest(server_path)?;
    let previous_count = manifest.collections.len();
    manifest
        .collections
        .retain(|collection| !removed_ids.contains(collection));
    if manifest.collections.len() == previous_count {
        println!("{}", language.text("steam.workshop.remove.nothing")?);
        return Ok(());
    }

    let remaining_items = if manifest.collections.is_empty() {
        HashSet::new()
    } else {
        fetch_collection_items(config, &manifest.collections)?
            .into_iter()
            .collect::<HashSet<_>>()
    };
    let orphaned_items = manifest
        .plugins
        .keys()
        .filter(|item_id| !remaining_items.contains(*item_id))
        .cloned()
        .collect::<Vec<_>>();
    if !confirm(language, "steam.workshop.remove.confirm")? {
        println!("{}", language.text("steam.workshop.remove.cancelled")?);
        return Ok(());
    }

    for item_id in orphaned_items {
        if let Some(plugin) = manifest.plugins.remove(&item_id) {
            for name in plugin.files {
                remove_regular_file(&destination.join(name))?;
            }
        }
    }
    write_manifest(server_path, &manifest)?;
    if let Some(collection_id) = manifest.collections.last() {
        write_collection_launcher(server_path, collection_id)?;
    } else {
        remove_collection_launcher(server_path)?;
    }
    println!(
        "{}",
        language.text("steam.workshop.remove.collection.done")?
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

fn remove_collection_launcher(server_path: &Path) -> AppResult<()> {
    let launcher = server_path.join("start-workshop.sh");
    let contents = match fs::read_to_string(&launcher) {
        Ok(contents) => contents,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(Box::new(error)),
    };
    let begin = contents.find(COLLECTION_BEGIN);
    let end = contents.find(COLLECTION_END);
    let (Some(begin), Some(end)) = (begin, end) else {
        return Err(input_error(
            "start-workshop.sh does not contain a complete GSBuilder collection block",
        ));
    };
    if begin >= end {
        return Err(input_error(
            "start-workshop.sh contains an invalid GSBuilder collection block",
        ));
    }
    let end = end + COLLECTION_END.len();
    let mut updated = String::with_capacity(contents.len());
    updated.push_str(&contents[..begin]);
    updated.push_str("exec \"$launcher\" -game left4dead2 \"$@\"");
    updated.push_str(&contents[end..]);
    fs::write(launcher, updated)?;
    Ok(())
}

fn confirm(language: &Language, key: &str) -> AppResult<bool> {
    print!("{}", language.text(key)?);
    io::stdout().flush()?;
    let mut answer = String::new();
    if io::stdin().read_line(&mut answer)? == 0 {
        return Err(input_error(
            language.text("steam.workshop.confirm.required")?,
        ));
    }
    match answer.trim().to_ascii_lowercase().as_str() {
        "" | "n" | "no" | "いいえ" => Ok(false),
        "y" | "yes" | "はい" => Ok(true),
        _ => Err(input_error(
            language.text("steam.workshop.confirm.invalid")?,
        )),
    }
}

fn confirm_for_item(language: &Language, key: &str, item_id: &str) -> AppResult<bool> {
    print!("{}", language.format(key, &[("id", item_id)])?);
    io::stdout().flush()?;
    let mut answer = String::new();
    if io::stdin().read_line(&mut answer)? == 0 {
        return Err(input_error(
            language.text("steam.workshop.confirm.required")?,
        ));
    }
    match answer.trim().to_ascii_lowercase().as_str() {
        "" | "n" | "no" | "いいえ" => Ok(false),
        "y" | "yes" | "はい" => Ok(true),
        _ => Err(input_error(
            language.text("steam.workshop.confirm.invalid")?,
        )),
    }
}

fn remove_regular_file(path: &Path) -> AppResult<()> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_file() => {
            Err(input_error(format!(
                "Workshop file is not a regular file: {}",
                path.display()
            )))
        }
        Ok(_) => {
            fs::remove_file(path)?;
            Ok(())
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(Box::new(error)),
    }
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
    filetype: u32,
}

struct WorkshopReference {
    id: String,
    force_collection: bool,
}

#[derive(Deserialize)]
struct PublishedFileDetailsResponse {
    response: PublishedFileDetailsContainer,
}

#[derive(Deserialize)]
struct PublishedFileDetailsContainer {
    publishedfiledetails: Vec<PublishedFileDetails>,
}

#[derive(Clone, Deserialize)]
struct PublishedFileDetails {
    publishedfileid: String,
    #[serde(default)]
    result: u32,
    #[serde(default)]
    title: String,
    #[serde(default)]
    time_updated: u64,
}

#[derive(Default, Deserialize, Serialize)]
#[serde(default)]
struct WorkshopManifest {
    collections: Vec<String>,
    plugins: HashMap<String, PluginRecord>,
}

#[derive(Default, Deserialize, Serialize)]
#[serde(default)]
struct PluginRecord {
    title: String,
    time_updated: u64,
    files: Vec<String>,
}

fn fetch_collection_items(config: &Config, collection_ids: &[String]) -> AppResult<Vec<String>> {
    let references = collection_ids
        .iter()
        .map(|id| WorkshopReference {
            id: id.clone(),
            force_collection: true,
        })
        .collect();
    resolve_workshop_references(config, references)
}

fn parse_workshop_references(
    input: &str,
    force_collection: bool,
) -> AppResult<Vec<WorkshopReference>> {
    let mut references = Vec::new();
    let mut seen = HashSet::new();
    for token in input
        .split(',')
        .map(str::trim)
        .filter(|token| !token.is_empty())
    {
        let (token, explicitly_collection) = token
            .strip_prefix("collection:")
            .map_or((token, false), |value| (value.trim(), true));
        let id = parse_workshop_id(token)?;
        if seen.insert(id.clone()) {
            references.push(WorkshopReference {
                id,
                force_collection: force_collection || explicitly_collection,
            });
        }
    }
    if references.is_empty() {
        return Err(input_error(
            "Enter at least one Workshop item or collection ID/URL",
        ));
    }
    Ok(references)
}

fn resolve_workshop_references(
    config: &Config,
    references: Vec<WorkshopReference>,
) -> AppResult<Vec<String>> {
    let mut pending = VecDeque::from(references);
    let mut visited_collections = HashSet::new();
    let mut item_set = HashSet::new();
    let mut items = Vec::new();

    while let Some(reference) = pending.pop_front() {
        if visited_collections.contains(&reference.id) {
            continue;
        }
        let children = match fetch_collection_batch(config, std::slice::from_ref(&reference.id)) {
            Ok(collections) => collections
                .into_iter()
                .next()
                .filter(|collection| collection.result == 1)
                .map(|collection| collection.children)
                .filter(|children| !children.is_empty()),
            Err(error) => {
                eprintln!(
                    "Warning: Could not query Workshop collection {} through Steam's API: {error}",
                    reference.id
                );
                None
            }
        };
        let children = match children {
            Some(children) => Some(children),
            None => match fetch_collection_page_children(config, &reference.id) {
                Ok(children) => children,
                Err(error) if reference.force_collection => return Err(error),
                Err(error) => {
                    eprintln!(
                        "Warning: Could not inspect Workshop page {}: {error}; treating it as an item",
                        reference.id
                    );
                    None
                }
            },
        };

        if let Some(children) = children {
            if !visited_collections.insert(reference.id.clone()) {
                continue;
            }
            if visited_collections.len() > MAX_COLLECTIONS {
                return Err(input_error(format!(
                    "Workshop collection graph exceeds the supported limit of {MAX_COLLECTIONS} collections"
                )));
            }
            for child in children {
                validate_workshop_id(&child.publishedfileid)?;
                match child.filetype {
                    0 => {
                        if item_set.insert(child.publishedfileid.clone()) {
                            items.push(child.publishedfileid);
                        }
                    }
                    2 => pending.push_back(WorkshopReference {
                        id: child.publishedfileid,
                        force_collection: true,
                    }),
                    _ => {}
                }
            }
        } else if reference.force_collection {
            return Err(input_error(format!(
                "Workshop ID {} is not a readable public collection",
                reference.id
            )));
        } else if item_set.insert(reference.id.clone()) {
            items.push(reference.id);
        }

        if items.len() > MAX_COLLECTION_ITEMS {
            return Err(input_error(format!(
                "Workshop references contain more than the supported {MAX_COLLECTION_ITEMS} items"
            )));
        }
    }

    if items.is_empty() {
        return Err(input_error(
            "Workshop references contain no downloadable items",
        ));
    }
    Ok(items)
}

fn fetch_collection_page_children(
    config: &Config,
    collection_id: &str,
) -> AppResult<Option<Vec<CollectionChild>>> {
    let url = format!("https://steamcommunity.com/sharedfiles/filedetails/?id={collection_id}");
    let response = http_client(config)?.get(url).send()?.error_for_status()?;
    let body = read_bounded_response(response)?;
    let html = String::from_utf8_lossy(&body);
    Ok(parse_collection_page_children(&html))
}

fn parse_collection_page_children(html: &str) -> Option<Vec<CollectionChild>> {
    let marker = html.find("id=\"mainContentsCollection\"")?;
    let open_start = html[..marker].rfind("<div")?;
    let open_end = html[open_start..].find('>')? + open_start;
    if marker > open_end {
        return None;
    }

    let mut depth = 1_usize;
    let mut cursor = open_end + 1;
    let mut children = Vec::new();
    let mut seen = HashSet::new();
    while cursor < html.len() {
        let next_open = html[cursor..].find("<div").map(|index| cursor + index);
        let next_close = html[cursor..].find("</div").map(|index| cursor + index);
        let (position, is_open) = match (next_open, next_close) {
            (Some(open), Some(close)) if open < close => (open, true),
            (Some(_), Some(close)) => (close, false),
            (Some(open), None) => (open, true),
            (None, Some(close)) => (close, false),
            (None, None) => return None,
        };
        if is_open {
            depth += 1;
            let end = html[position..].find('>')? + position;
            let tag = &html[position..=end];
            if let Some(id) = sharedfile_id(tag)
                && has_class(tag, "collectionItem")
                && seen.insert(id.to_owned())
            {
                children.push(CollectionChild {
                    publishedfileid: id.to_owned(),
                    filetype: 0,
                });
            }
            cursor = end + 1;
        } else {
            depth -= 1;
            if depth == 0 {
                break;
            }
            cursor = position + "</div".len();
        }
    }
    (!children.is_empty()).then_some(children)
}

fn sharedfile_id(tag: &str) -> Option<&str> {
    let start = tag.find("id=\"sharedfile_")? + "id=\"sharedfile_".len();
    let end = tag[start..].find('"')? + start;
    let id = &tag[start..end];
    (id.bytes().all(|byte| byte.is_ascii_digit()) && !id.is_empty()).then_some(id)
}

fn has_class(tag: &str, expected: &str) -> bool {
    let Some(start) = tag.find("class=\"") else {
        return false;
    };
    let start = start + "class=\"".len();
    let Some(end) = tag[start..].find('"') else {
        return false;
    };
    tag[start..start + end]
        .split_ascii_whitespace()
        .any(|class| class == expected)
}

fn fetch_collection_batch(
    config: &Config,
    collection_ids: &[String],
) -> AppResult<Vec<CollectionDetails>> {
    let mut form = vec![(
        "collectioncount".to_owned(),
        collection_ids.len().to_string(),
    )];
    form.extend(
        collection_ids
            .iter()
            .enumerate()
            .map(|(index, id)| (format!("publishedfileids[{index}]"), id.clone())),
    );
    let response = http_client(config)?
        .post(COLLECTION_DETAILS_URL)
        .form(&form)
        .send()?
        .error_for_status()?;
    let body = read_bounded_response(response)?;
    parse_collection_response(&body, collection_ids.len())
}

fn parse_collection_response(
    body: &[u8],
    expected_count: usize,
) -> AppResult<Vec<CollectionDetails>> {
    let parsed: CollectionApiResponse = serde_json::from_slice(body)?;
    if parsed.response.collectiondetails.len() != expected_count {
        return Err(input_error(
            "Steam did not return details for every requested Workshop collection",
        ));
    }
    Ok(parsed.response.collectiondetails)
}

fn fetch_published_file_details(
    config: &Config,
    item_ids: &[String],
) -> AppResult<Vec<PublishedFileDetails>> {
    let mut details_by_id = HashMap::with_capacity(item_ids.len());
    for batch in item_ids.chunks(API_BATCH_SIZE) {
        let mut form = vec![("itemcount".to_owned(), batch.len().to_string())];
        form.extend(
            batch
                .iter()
                .enumerate()
                .map(|(index, id)| (format!("publishedfileids[{index}]"), id.clone())),
        );
        let response = http_client(config)?
            .post(PUBLISHED_FILE_DETAILS_URL)
            .form(&form)
            .send()?
            .error_for_status()?;
        let body = read_bounded_response(response)?;
        let parsed: PublishedFileDetailsResponse = serde_json::from_slice(&body)?;
        for detail in parsed.response.publishedfiledetails {
            if detail.result != 1 {
                return Err(input_error(format!(
                    "Steam could not load Workshop item {} (result {})",
                    detail.publishedfileid, detail.result
                )));
            }
            details_by_id.insert(detail.publishedfileid.clone(), detail);
        }
    }

    item_ids
        .iter()
        .map(|item_id| {
            details_by_id.remove(item_id).ok_or_else(|| {
                input_error(format!(
                    "Steam did not return details for Workshop item {item_id}"
                ))
            })
        })
        .collect()
}

fn read_bounded_response(response: reqwest::blocking::Response) -> AppResult<Vec<u8>> {
    if response
        .content_length()
        .is_some_and(|length| length > MAX_COLLECTION_RESPONSE_SIZE)
    {
        return Err(input_error(
            "Steam Workshop response exceeds the 2 MiB size limit",
        ));
    }
    let mut body = Vec::new();
    response
        .take(MAX_COLLECTION_RESPONSE_SIZE + 1)
        .read_to_end(&mut body)?;
    if body.len() as u64 > MAX_COLLECTION_RESPONSE_SIZE {
        return Err(input_error(
            "Steam Workshop response exceeds the 2 MiB size limit",
        ));
    }
    Ok(body)
}

fn read_manifest(server_path: &Path) -> AppResult<WorkshopManifest> {
    let path = server_path.join("addons.lst");
    let manifest = match fs::read(&path) {
        Ok(contents) => serde_json::from_slice(&contents)?,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => WorkshopManifest::default(),
        Err(error) => return Err(Box::new(error)),
    };
    validate_manifest(&manifest)?;
    Ok(manifest)
}

fn validate_manifest(manifest: &WorkshopManifest) -> AppResult<()> {
    for collection_id in &manifest.collections {
        validate_workshop_id(collection_id)?;
    }
    let mut owners = HashMap::new();
    for (item_id, plugin) in &manifest.plugins {
        validate_workshop_id(item_id)?;
        for name in &plugin.files {
            if !is_vpk_filename(name) {
                return Err(input_error(format!(
                    "Invalid VPK filename in addons.lst: {name}"
                )));
            }
            if owners.insert(name, item_id).is_some() {
                return Err(input_error(format!(
                    "addons.lst assigns VPK file {name} to multiple Workshop items"
                )));
            }
        }
    }
    Ok(())
}

fn is_vpk_filename(name: &str) -> bool {
    !name.is_empty()
        && name != "."
        && name != ".."
        && !name.contains(['/', '\\'])
        && Path::new(name)
            .extension()
            .is_some_and(|extension| extension.eq_ignore_ascii_case("vpk"))
}

fn plugin_is_current(plugin: Option<&PluginRecord>, time_updated: u64, directory: &Path) -> bool {
    plugin.is_some_and(|plugin| {
        plugin.time_updated == time_updated
            && !plugin.files.is_empty()
            && plugin.files.iter().all(|name| {
                fs::symlink_metadata(directory.join(name)).is_ok_and(|meta| meta.is_file())
            })
    })
}

fn write_manifest(server_path: &Path, manifest: &WorkshopManifest) -> AppResult<()> {
    validate_manifest(manifest)?;
    let path = server_path.join("addons.lst");
    let temporary_path = server_path.join(format!("addons.lst.{}.tmp", std::process::id()));
    fs::write(&temporary_path, serde_json::to_vec_pretty(manifest)?)?;
    fs::rename(temporary_path, path)?;
    Ok(())
}

fn install_vpk_updates(
    updates: &[(&PublishedFileDetails, PathBuf)],
    manifest: &mut WorkshopManifest,
    destination: &Path,
    mut confirm: impl FnMut(&str, &str) -> AppResult<bool>,
) -> AppResult<usize> {
    let mut new_files = HashMap::<String, Vec<(PathBuf, String)>>::new();
    let mut owners = HashMap::<String, String>::new();
    for (item, source) in updates {
        let mut source_paths = Vec::new();
        collect_vpk_files(source, &mut source_paths)?;
        if source_paths.is_empty() {
            return Err(input_error(format!(
                "Workshop item {} contains no VPK files",
                item.publishedfileid
            )));
        }
        let mut paths = Vec::new();
        for path in &source_paths {
            let name = vpk_output_filename(path)?;
            if let Some(owner) = owners.insert(name.clone(), item.publishedfileid.clone()) {
                return Err(input_error(format!(
                    "Workshop items {owner} and {} contain VPK files with the same name: {name}",
                    item.publishedfileid
                )));
            }
            paths.push((path.clone(), name));
        }
        new_files.insert(item.publishedfileid.clone(), paths);
    }

    let mut declined_updates = HashSet::new();
    for (item_id, paths) in &new_files {
        let mut overwrites_existing = false;
        for (_, name) in paths {
            if let Some((other_item, _)) = manifest.plugins.iter().find(|(other_id, plugin)| {
                *other_id != item_id && plugin.files.iter().any(|file| file == name)
            }) {
                return Err(input_error(format!(
                    "Workshop item {item_id} conflicts with item {other_item} over VPK file {name}"
                )));
            }
            let target = destination.join(name);
            match fs::symlink_metadata(&target) {
                Ok(metadata) => {
                    if metadata.file_type().is_symlink() || !metadata.is_file() {
                        return Err(input_error(format!(
                            "Workshop VPK destination is not a regular file: {}",
                            target.display()
                        )));
                    }
                    overwrites_existing = true;
                }
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => return Err(Box::new(error)),
            }
        }
        if overwrites_existing && !confirm("steam.workshop.overwrite.prompt", item_id)? {
            declined_updates.insert(item_id.clone());
        }
    }
    new_files.retain(|item_id, _| !declined_updates.contains(item_id));

    fs::create_dir_all(destination)?;
    for (item_id, paths) in &new_files {
        let mut names = Vec::with_capacity(paths.len());
        for (source, name) in paths {
            fs::copy(source, destination.join(name))?;
            names.push(name.to_owned());
        }

        if let Some(previous) = manifest.plugins.get(item_id) {
            let stale_names = previous
                .files
                .iter()
                .filter(|old_name| !names.iter().any(|name| name == *old_name))
                .filter(|old_name| destination.join(old_name).is_file())
                .cloned()
                .collect::<Vec<_>>();
            let delete_stale =
                stale_names.is_empty() || confirm("steam.workshop.delete_stale.prompt", item_id)?;
            if !delete_stale {
                names.extend(stale_names);
            }
            for old_name in &previous.files {
                if delete_stale && !names.iter().any(|name| name == old_name) {
                    let old_path = destination.join(old_name);
                    match fs::symlink_metadata(&old_path) {
                        Ok(metadata)
                            if metadata.file_type().is_symlink() || !metadata.is_file() =>
                        {
                            return Err(input_error(format!(
                                "Managed Workshop VPK is not a regular file: {}",
                                old_path.display()
                            )));
                        }
                        Ok(_) => fs::remove_file(old_path)?,
                        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                        Err(error) => return Err(Box::new(error)),
                    }
                }
            }
        }

        let detail = updates
            .iter()
            .find(|(detail, _)| detail.publishedfileid == *item_id)
            .map(|(detail, _)| detail)
            .ok_or_else(|| input_error(format!("Missing metadata for Workshop item {item_id}")))?;
        manifest.plugins.insert(
            item_id.clone(),
            PluginRecord {
                title: detail.title.clone(),
                time_updated: detail.time_updated,
                files: names,
            },
        );
    }
    Ok(new_files.values().map(Vec::len).sum())
}

fn download_items(
    config: &Config,
    server_path: &Path,
    workshop_ids: &[String],
    username: &str,
) -> AppResult<HashMap<String, PathBuf>> {
    if workshop_ids.is_empty() {
        return Ok(HashMap::new());
    }
    let steamcmd = config.executable_path(&config.steam.steamcmd_executable);
    let mut args = vec![
        "+force_install_dir".to_owned(),
        server_path.to_string_lossy().into_owned(),
        "+login".to_owned(),
        username.to_owned(),
    ];
    for workshop_id in workshop_ids {
        println!("Downloading Workshop item: {workshop_id}");
        args.extend([
            "+workshop_download_item".to_owned(),
            L4D2_WORKSHOP_APP_ID.to_owned(),
            workshop_id.clone(),
        ]);
        if config.steam.validate_install {
            args.push("validate".to_owned());
        }
    }
    args.push("+quit".to_owned());
    let status = super::run_steamcmd(&steamcmd, &args, &config.steam.run_as_user)?;
    if !status.success() {
        return Err(input_error(format!(
            "SteamCMD failed to download Workshop items: {status}"
        )));
    }

    let user_home = steam_user_home(&config.steam.run_as_user)?;
    workshop_ids
        .iter()
        .map(|workshop_id| {
            workshop_content_directory(&steamcmd, &user_home, server_path, workshop_id)
                .map(|path| (workshop_id.clone(), path))
                .ok_or_else(|| {
                    input_error(format!(
                        "SteamCMD reported success, but Workshop item {workshop_id} was not found in its content directory"
                    ))
                })
        })
        .collect()
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

fn parse_workshop_id(value: &str) -> AppResult<String> {
    if value.bytes().all(|byte| byte.is_ascii_digit()) {
        validate_workshop_id(value)?;
        return Ok(value.to_owned());
    }

    let url = reqwest::Url::parse(value).map_err(|_| {
        input_error("Workshop input must be a numeric ID or a Steam Community Workshop URL")
    })?;
    if !matches!(url.scheme(), "http" | "https")
        || !matches!(
            url.host_str(),
            Some("steamcommunity.com" | "www.steamcommunity.com")
        )
        || url.port().is_some()
        || !url.username().is_empty()
        || url.password().is_some()
        || !matches!(
            url.path(),
            "/sharedfiles/filedetails"
                | "/sharedfiles/filedetails/"
                | "/workshop/filedetails"
                | "/workshop/filedetails/"
        )
    {
        return Err(input_error(
            "Unsupported Workshop URL; use a Steam Community item or collection details URL",
        ));
    }

    let mut ids = url
        .query_pairs()
        .filter(|(key, _)| key == "id")
        .map(|(_, id)| id.into_owned());
    let id = ids
        .next()
        .ok_or_else(|| input_error("Steam Workshop URL must include a numeric ?id= parameter"))?;
    if ids.next().is_some() {
        return Err(input_error(
            "Steam Workshop URL must include exactly one id parameter",
        ));
    }
    validate_workshop_id(&id)?;
    Ok(id)
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

#[cfg(test)]
fn install_vpks(source: &Path, destination: &Path) -> AppResult<usize> {
    install_vpk_sources(&[source.to_path_buf()], destination)
}

fn install_direct_items(
    item_ids: &[String],
    downloads: &HashMap<String, PathBuf>,
    destination: &Path,
    protected_names: &HashSet<String>,
    mut confirm: impl FnMut(&str, &str) -> AppResult<bool>,
) -> AppResult<usize> {
    let mut installs = Vec::new();
    let mut expected_names = HashSet::new();
    for item_id in item_ids {
        let source = downloads.get(item_id).ok_or_else(|| {
            input_error(format!("Workshop item {item_id} has no downloaded content"))
        })?;
        let mut files = Vec::new();
        collect_vpk_files(source, &mut files)?;
        files.sort();
        if files.is_empty() {
            return Err(input_error(format!(
                "Workshop item {item_id} contains no VPK files"
            )));
        }
        let single_file = files.len() == 1;
        let mut names = Vec::with_capacity(files.len());
        for (index, file) in files.into_iter().enumerate() {
            let filename = vpk_output_filename(&file)?;
            let target_name = if single_file {
                format!("{item_id}.vpk")
            } else {
                format!("{item_id}_{index}_{filename}")
            };
            if !expected_names.insert(target_name.clone()) {
                return Err(input_error(format!(
                    "Workshop items contain a conflicting output filename: {target_name}"
                )));
            }
            if protected_names.contains(&target_name) {
                return Err(input_error(format!(
                    "Workshop VPK {target_name} is managed by a registered collection"
                )));
            }
            names.push((file, target_name));
        }
        let has_existing_target = names.iter().try_fold(false, |found, (_, name)| {
            if found {
                return Ok(true);
            }
            match fs::symlink_metadata(destination.join(name)) {
                Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_file() => {
                    Err(input_error(format!(
                        "Workshop VPK destination is not a regular file: {}",
                        destination.join(name).display()
                    )))
                }
                Ok(_) => Ok(true),
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
                Err(error) => Err(Box::new(error)),
            }
        })?;
        if has_existing_target && !confirm("steam.workshop.overwrite.prompt", item_id)? {
            println!("Skipped Workshop item {item_id}.");
            continue;
        }
        installs.push((item_id.clone(), names));
    }

    fs::create_dir_all(destination)?;
    let mut installed = 0;
    for (item_id, files) in &installs {
        for (source, name) in files {
            let target = destination.join(name);
            match fs::symlink_metadata(&target) {
                Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_file() => {
                    return Err(input_error(format!(
                        "Workshop VPK destination is not a regular file: {}",
                        target.display()
                    )));
                }
                Ok(_) => {}
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => return Err(Box::new(error)),
            }
            fs::copy(source, &target)?;
        }
        let current_names = files
            .iter()
            .map(|(_, name)| name.as_str())
            .collect::<HashSet<_>>();
        let mut stale_files = Vec::new();
        for entry in fs::read_dir(destination)? {
            let entry = entry?;
            let name = entry.file_name().to_str().map(str::to_owned);
            let file_type = entry.file_type()?;
            if file_type.is_file()
                && let Some(name) = name
                && is_direct_item_filename(&name, item_id)
                && !protected_names.contains(&name)
                && !current_names.contains(name.as_str())
            {
                stale_files.push((entry.path(), name));
            }
        }
        if !stale_files.is_empty() && confirm("steam.workshop.delete_stale.prompt", item_id)? {
            for (path, _) in stale_files {
                fs::remove_file(path)?;
            }
        }
        installed += files.len();
    }
    Ok(installed)
}

fn is_direct_item_filename(filename: &str, item_id: &str) -> bool {
    filename == format!("{item_id}.vpk")
        || filename
            .strip_prefix(&format!("{item_id}_"))
            .is_some_and(|suffix| suffix.ends_with(".vpk"))
}

#[cfg(test)]
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
            && (path
                .extension()
                .is_some_and(|extension| extension.eq_ignore_ascii_case("vpk"))
                || is_legacy_vpk(&path)?)
        {
            output.push(path);
        }
    }
    Ok(())
}

fn is_legacy_vpk(path: &Path) -> AppResult<bool> {
    let is_legacy_file = path
        .file_name()
        .and_then(|name| name.to_str())
        .is_some_and(|name| name.to_ascii_lowercase().ends_with("_legacy.bin"));
    if !is_legacy_file {
        return Ok(false);
    }

    let mut file = fs::File::open(path)?;
    let mut signature = [0; 4];
    match file.read_exact(&mut signature) {
        Ok(()) => Ok(signature == [0x34, 0x12, 0xaa, 0x55]),
        Err(error) if error.kind() == std::io::ErrorKind::UnexpectedEof => Ok(false),
        Err(error) => Err(Box::new(error)),
    }
}

fn update_collection_launcher(contents: &str, collection_id: &str) -> AppResult<String> {
    let begin = contents.find(COLLECTION_BEGIN);
    let end = contents.find(COLLECTION_END);
    match (begin, end) {
        (Some(begin), Some(end)) if begin < end => {
            let block = collection_launcher_block(collection_id, "");
            let end = end + COLLECTION_END.len();
            let mut updated = String::with_capacity(contents.len() + block.len());
            updated.push_str(&contents[..begin]);
            updated.push_str(&block);
            updated.push_str(&contents[end..]);
            Ok(updated)
        }
        (None, None) => {
            if contents.is_empty() {
                return Ok(format!(
                    "#!/bin/sh\nset -eu\n\nserver_root=$(CDPATH= cd -- \"$(dirname -- \"$0\")\" && pwd)\nif [ -x \"$server_root/srcds_run\" ]; then\n    launcher=\"$server_root/srcds_run\"\nelif [ -x \"$server_root/srcds_run.sh\" ]; then\n    launcher=\"$server_root/srcds_run.sh\"\nelse\n    printf '%s\\n' \"Could not find srcds_run in $server_root\" >&2\n    exit 1\nfi\n\n{}\n",
                    collection_launcher_block(collection_id, "")
                ));
            }

            let expected = "exec \"$launcher\" -game left4dead2 \"$@\"";
            let matching_lines = contents
                .lines()
                .filter(|line| line.trim() == expected)
                .count();
            if matching_lines != 1 {
                return Err(input_error(
                    "start-workshop.sh exists but has no unique standard Left 4 Dead 2 launcher command; update it to end with `exec \"$launcher\" -game left4dead2 \"$@\"` and retry",
                ));
            }

            let mut updated = String::with_capacity(contents.len() + 100);
            for line in contents.split_inclusive('\n') {
                let without_newline = line.strip_suffix('\n').unwrap_or(line);
                let line_content = without_newline
                    .strip_suffix('\r')
                    .unwrap_or(without_newline);
                if line_content.trim() == expected {
                    let indent =
                        &line_content[..line_content.len() - line_content.trim_start().len()];
                    updated.push_str(&collection_launcher_block(collection_id, indent));
                    if line.ends_with('\n') {
                        updated.push('\n');
                    }
                } else {
                    updated.push_str(line);
                }
            }
            Ok(updated)
        }
        _ => Err(input_error(
            "start-workshop.sh contains an incomplete GSBuilder collection block",
        )),
    }
}

fn collection_launcher_block(collection_id: &str, indent: &str) -> String {
    format!(
        "{indent}{COLLECTION_BEGIN}\n{indent}exec \"$launcher\" -game left4dead2 \"$@\" +host_workshop_collection {collection_id}\n{indent}{COLLECTION_END}"
    )
}

#[cfg(test)]
mod tests {
    use super::{
        COLLECTION_BEGIN, PluginRecord, PublishedFileDetails, WorkshopManifest,
        install_direct_items, install_vpk_updates, install_vpks, parse_collection_page_children,
        parse_collection_response, parse_workshop_id, parse_workshop_references, plugin_is_current,
        read_manifest, remove_collection_launcher, update_collection_launcher, validate_manifest,
        validate_workshop_id, write_collection_launcher, write_manifest,
    };
    use std::collections::{HashMap, HashSet};
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
    fn accepts_workshop_ids_and_steam_item_or_collection_urls() {
        assert_eq!(parse_workshop_id("123456").unwrap(), "123456");
        assert_eq!(
            parse_workshop_id("https://steamcommunity.com/sharedfiles/filedetails/?id=123456")
                .unwrap(),
            "123456"
        );
        assert_eq!(
            parse_workshop_id(
                "https://www.steamcommunity.com/workshop/filedetails/?foo=bar&id=987654"
            )
            .unwrap(),
            "987654"
        );
        assert_eq!(
            parse_workshop_id("http://steamcommunity.com/sharedfiles/filedetails?id=123456")
                .unwrap(),
            "123456"
        );
    }

    #[test]
    fn rejects_invalid_or_unsupported_workshop_urls() {
        assert!(parse_workshop_id("https://example.com/sharedfiles/filedetails/?id=123").is_err());
        assert!(parse_workshop_id("https://steamcommunity.com/sharedfiles/filedetails/").is_err());
        assert!(
            parse_workshop_id("https://steamcommunity.com/sharedfiles/filedetails/?id=123&id=456")
                .is_err()
        );
        assert!(
            parse_workshop_id("https://steamcommunity.com/sharedfiles/filedetails/?id=../123")
                .is_err()
        );
        assert!(
            parse_workshop_id("https://steamcommunity.com/id/someuser/myworkshopfiles/").is_err()
        );
    }

    #[test]
    fn parses_multiple_item_and_collection_references() {
        let references = parse_workshop_references(
            "123456, https://steamcommunity.com/sharedfiles/filedetails/?id=234567, collection:345678",
            false,
        )
        .unwrap();
        assert_eq!(
            references
                .iter()
                .map(|reference| reference.id.as_str())
                .collect::<Vec<_>>(),
            ["123456", "234567", "345678"]
        );
        assert!(!references[0].force_collection);
        assert!(references[2].force_collection);
        let forced = parse_workshop_references("123456,234567", true).unwrap();
        assert!(forced.iter().all(|reference| reference.force_collection));
        assert!(
            parse_workshop_references("123456,123456", false)
                .unwrap()
                .len()
                == 1
        );
    }

    #[test]
    fn extracts_items_only_from_the_workshop_collection_container() {
        let html = r#"
            <div id="mainContentsCollection">
              <div id="sharedfile_567089407" class="collectionItem"></div>
              <div class="collectionItem featured" id="sharedfile_3773812908"></div>
            </div>
            <div id="mainContentsRelated">
              <div id="sharedfile_999999999" class="collectionItem"></div>
            </div>
        "#;
        let children = parse_collection_page_children(html).unwrap();
        assert_eq!(
            children
                .iter()
                .map(|child| child.publishedfileid.as_str())
                .collect::<Vec<_>>(),
            ["567089407", "3773812908"]
        );
        assert!(parse_collection_page_children("<html></html>").is_none());
    }

    #[test]
    fn parses_collection_children_with_item_and_subcollection_types() {
        let body = br#"{"response":{"collectiondetails":[{"result":1,"children":[{"publishedfileid":"123","filetype":0},{"publishedfileid":"456","filetype":2}]}]}}"#;
        let collections = parse_collection_response(body, 1).unwrap();
        assert_eq!(collections[0].result, 1);
        assert_eq!(collections[0].children[0].publishedfileid, "123");
        assert_eq!(collections[0].children[0].filetype, 0);
        assert_eq!(collections[0].children[1].filetype, 2);
    }

    #[test]
    fn rejects_unexpected_collection_response_count() {
        assert!(
            parse_collection_response(
                br#"{"response":{"collectiondetails":[{"result":1,"children":[]}]}}"#,
                2
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

    #[test]
    fn adds_collection_to_an_existing_standard_workshop_launcher() {
        let existing = "#!/bin/sh\nset -eu\n\nserver_root=/srv/l4d2\nlauncher=\"$server_root/srcds_run\"\n\nexec \"$launcher\" -game left4dead2 \"$@\"\n";
        let updated = update_collection_launcher(existing, "123456").unwrap();

        assert!(updated.starts_with("#!/bin/sh\nset -eu\n"));
        assert!(updated.contains("server_root=/srv/l4d2"));
        assert!(updated.contains("launcher=\"$server_root/srcds_run\""));
        assert!(updated.contains(
            "exec \"$launcher\" -game left4dead2 \"$@\" +host_workshop_collection 123456"
        ));
        assert!(!updated.contains("exec \"$launcher\" -game left4dead2 \"$@\"\n"));
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

    #[test]
    fn installs_steam_legacy_vpk_files_with_a_vpk_extension() {
        let root = temp_dir();
        let source = root.join("source");
        let destination = root.join("server/addons/workshop");
        fs::create_dir_all(&source).unwrap();
        fs::write(
            source.join("10261711852140370800_legacy.bin"),
            [0x34, 0x12, 0xaa, 0x55, 1, 0, 0, 0],
        )
        .unwrap();
        let ids = vec!["3815658696".to_owned()];
        let downloads = HashMap::from([("3815658696".to_owned(), source)]);

        assert_eq!(
            install_direct_items(&ids, &downloads, &destination, &HashSet::new(), |_, _| {
                Ok(true)
            })
            .unwrap(),
            1
        );
        assert_eq!(
            fs::read(destination.join("3815658696.vpk")).unwrap(),
            [0x34, 0x12, 0xaa, 0x55, 1, 0, 0, 0]
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn batch_item_install_uses_id_names_updates_vpks_and_removes_stale_parts() {
        let root = temp_dir();
        let source = root.join("source");
        let destination = root.join("server/addons/workshop");
        fs::create_dir_all(&source).unwrap();
        fs::write(source.join("first.vpk"), b"first").unwrap();
        fs::write(source.join("second.vpk"), b"second").unwrap();
        let ids = vec!["123456".to_owned()];
        let downloads = HashMap::from([("123456".to_owned(), source.clone())]);
        let protected = HashSet::new();
        assert_eq!(
            install_direct_items(&ids, &downloads, &destination, &protected, |_, _| Ok(true))
                .unwrap(),
            2
        );
        assert_eq!(
            fs::read(destination.join("123456_0_first.vpk")).unwrap(),
            b"first"
        );

        fs::remove_file(source.join("second.vpk")).unwrap();
        fs::write(source.join("first.vpk"), b"updated").unwrap();
        assert_eq!(
            install_direct_items(&ids, &downloads, &destination, &protected, |_, _| Ok(true))
                .unwrap(),
            1
        );
        assert_eq!(
            fs::read(destination.join("123456.vpk")).unwrap(),
            b"updated"
        );
        assert!(!destination.join("123456_0_first.vpk").exists());
        assert!(!destination.join("123456_1_second.vpk").exists());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn direct_item_install_respects_declined_overwrite_and_stale_file_removal() {
        let root = temp_dir();
        let source = root.join("source");
        let destination = root.join("server/addons/workshop");
        fs::create_dir_all(&source).unwrap();
        fs::write(source.join("first.vpk"), b"first").unwrap();
        fs::write(source.join("second.vpk"), b"second").unwrap();
        let ids = vec!["123456".to_owned()];
        let downloads = HashMap::from([("123456".to_owned(), source.clone())]);

        assert_eq!(
            install_direct_items(&ids, &downloads, &destination, &HashSet::new(), |_, _| {
                Ok(true)
            })
            .unwrap(),
            2
        );
        fs::remove_file(source.join("second.vpk")).unwrap();
        fs::write(source.join("first.vpk"), b"updated").unwrap();
        assert_eq!(
            install_direct_items(&ids, &downloads, &destination, &HashSet::new(), |_, _| {
                Ok(false)
            })
            .unwrap(),
            1
        );
        assert!(destination.join("123456_0_first.vpk").exists());
        assert!(destination.join("123456_1_second.vpk").exists());

        fs::write(source.join("first.vpk"), b"newer").unwrap();
        assert_eq!(
            install_direct_items(&ids, &downloads, &destination, &HashSet::new(), |_, _| {
                Ok(false)
            })
            .unwrap(),
            0
        );
        assert_eq!(
            fs::read(destination.join("123456.vpk")).unwrap(),
            b"updated"
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn direct_item_install_does_not_overwrite_collection_managed_files() {
        let root = temp_dir();
        let source = root.join("source");
        let destination = root.join("server/addons/workshop");
        fs::create_dir_all(&source).unwrap();
        fs::create_dir_all(&destination).unwrap();
        fs::write(source.join("addon.vpk"), b"new").unwrap();
        fs::write(destination.join("123456.vpk"), b"managed").unwrap();
        let ids = vec!["123456".to_owned()];
        let downloads = HashMap::from([("123456".to_owned(), source)]);
        let protected = HashSet::from(["123456.vpk".to_owned()]);
        assert!(
            install_direct_items(&ids, &downloads, &destination, &protected, |_, _| Ok(true))
                .is_err()
        );
        assert_eq!(
            fs::read(destination.join("123456.vpk")).unwrap(),
            b"managed"
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn updates_only_managed_vpks_and_persists_manifest_metadata() {
        let root = temp_dir();
        let destination = root.join("left4dead2/addons/workshop");
        let first_source = root.join("first");
        let second_source = root.join("second");
        fs::create_dir_all(&first_source).unwrap();
        fs::create_dir_all(&second_source).unwrap();
        fs::write(first_source.join("old.vpk"), b"old version").unwrap();
        fs::write(second_source.join("new.vpk"), b"new version").unwrap();

        let mut manifest = WorkshopManifest::default();
        let first_details = PublishedFileDetails {
            publishedfileid: "123".to_owned(),
            result: 1,
            title: "First title".to_owned(),
            time_updated: 10,
        };
        assert_eq!(
            install_vpk_updates(
                &[(&first_details, first_source)],
                &mut manifest,
                &destination,
                |_, _| Ok(true)
            )
            .unwrap(),
            1
        );
        assert_eq!(
            fs::read(destination.join("old.vpk")).unwrap(),
            b"old version"
        );

        let updated_details = PublishedFileDetails {
            publishedfileid: "123".to_owned(),
            result: 1,
            title: "Updated title".to_owned(),
            time_updated: 20,
        };
        assert_eq!(
            install_vpk_updates(
                &[(&updated_details, second_source)],
                &mut manifest,
                &destination,
                |_, _| Ok(true)
            )
            .unwrap(),
            1
        );
        assert!(!destination.join("old.vpk").exists());
        assert_eq!(
            fs::read(destination.join("new.vpk")).unwrap(),
            b"new version"
        );

        write_manifest(&root, &manifest).unwrap();
        let loaded = read_manifest(&root).unwrap();
        let plugin = &loaded.plugins["123"];
        assert_eq!(plugin.title, "Updated title");
        assert_eq!(plugin.time_updated, 20);
        assert_eq!(plugin.files, ["new.vpk"]);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn collection_sync_keeps_stale_files_or_skips_updates_when_confirmation_is_declined() {
        let root = temp_dir();
        let destination = root.join("left4dead2/addons/workshop");
        let first_source = root.join("first");
        let second_source = root.join("second");
        let third_source = root.join("third");
        fs::create_dir_all(&first_source).unwrap();
        fs::create_dir_all(&second_source).unwrap();
        fs::create_dir_all(&third_source).unwrap();
        fs::write(first_source.join("old.vpk"), b"old version").unwrap();
        fs::write(second_source.join("new.vpk"), b"new version").unwrap();
        fs::write(third_source.join("new.vpk"), b"newer version").unwrap();
        let first_details = PublishedFileDetails {
            publishedfileid: "123".to_owned(),
            result: 1,
            title: "First title".to_owned(),
            time_updated: 10,
        };
        let updated_details = PublishedFileDetails {
            publishedfileid: "123".to_owned(),
            result: 1,
            title: "Updated title".to_owned(),
            time_updated: 20,
        };
        let newer_details = PublishedFileDetails {
            publishedfileid: "123".to_owned(),
            result: 1,
            title: "Newer title".to_owned(),
            time_updated: 30,
        };
        let mut manifest = WorkshopManifest::default();
        install_vpk_updates(
            &[(&first_details, first_source)],
            &mut manifest,
            &destination,
            |_, _| Ok(true),
        )
        .unwrap();

        assert_eq!(
            install_vpk_updates(
                &[(&updated_details, second_source)],
                &mut manifest,
                &destination,
                |_, _| Ok(false)
            )
            .unwrap(),
            1
        );
        assert!(destination.join("old.vpk").exists());
        assert!(destination.join("new.vpk").exists());
        assert_eq!(manifest.plugins["123"].files, ["new.vpk", "old.vpk"]);

        assert_eq!(
            install_vpk_updates(
                &[(&newer_details, third_source)],
                &mut manifest,
                &destination,
                |_, _| Ok(false)
            )
            .unwrap(),
            0
        );
        assert_eq!(
            fs::read(destination.join("new.vpk")).unwrap(),
            b"new version"
        );
        assert_eq!(manifest.plugins["123"].time_updated, 20);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn collection_sync_asks_before_overwriting_untracked_vpk_files() {
        let root = temp_dir();
        let destination = root.join("left4dead2/addons/workshop");
        let source = root.join("download");
        fs::create_dir_all(&destination).unwrap();
        fs::create_dir_all(&source).unwrap();
        fs::write(destination.join("addon.vpk"), b"existing").unwrap();
        fs::write(source.join("addon.vpk"), b"workshop").unwrap();
        let details = PublishedFileDetails {
            publishedfileid: "123".to_owned(),
            result: 1,
            title: "Addon".to_owned(),
            time_updated: 10,
        };
        let mut manifest = WorkshopManifest::default();

        assert_eq!(
            install_vpk_updates(
                &[(&details, source.clone())],
                &mut manifest,
                &destination,
                |_, _| Ok(false)
            )
            .unwrap(),
            0
        );
        assert_eq!(
            fs::read(destination.join("addon.vpk")).unwrap(),
            b"existing"
        );

        assert_eq!(
            install_vpk_updates(
                &[(&details, source)],
                &mut manifest,
                &destination,
                |_, _| Ok(true)
            )
            .unwrap(),
            1
        );
        assert_eq!(
            fs::read(destination.join("addon.vpk")).unwrap(),
            b"workshop"
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn removing_collection_keeps_launcher_but_disables_collection_argument() {
        let root = temp_dir();
        fs::create_dir_all(&root).unwrap();
        write_collection_launcher(&root, "123456").unwrap();

        remove_collection_launcher(&root).unwrap();
        let launcher = fs::read_to_string(root.join("start-workshop.sh")).unwrap();
        assert!(launcher.contains("exec \"$launcher\" -game left4dead2 \"$@\""));
        assert!(!launcher.contains("+host_workshop_collection"));
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn collection_sync_converts_steam_legacy_vpks_and_tracks_the_vpk_name() {
        let root = temp_dir();
        let destination = root.join("left4dead2/addons/workshop");
        let source = root.join("download/3815658696");
        fs::create_dir_all(&source).unwrap();
        fs::write(
            source.join("10261711852140370800_legacy.bin"),
            [0x34, 0x12, 0xaa, 0x55, 1, 0, 0, 0],
        )
        .unwrap();
        let details = PublishedFileDetails {
            publishedfileid: "3815658696".to_owned(),
            result: 1,
            title: "Legacy addon".to_owned(),
            time_updated: 10,
        };
        let mut manifest = WorkshopManifest::default();

        assert_eq!(
            install_vpk_updates(
                &[(&details, source)],
                &mut manifest,
                &destination,
                |_, _| { Ok(true) }
            )
            .unwrap(),
            1
        );
        assert!(destination.join("10261711852140370800.vpk").is_file());
        assert_eq!(
            manifest.plugins["3815658696"].files,
            ["10261711852140370800.vpk"]
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn reads_reference_manifest_entries_without_file_lists() {
        let root = temp_dir();
        fs::create_dir_all(&root).unwrap();
        fs::write(
            root.join("addons.lst"),
            r#"{"collections":["100"],"plugins":{"200":{"title":"Example","time_updated":30}}}"#,
        )
        .unwrap();
        let manifest = read_manifest(&root).unwrap();
        assert_eq!(manifest.collections, ["100"]);
        assert_eq!(manifest.plugins["200"].title, "Example");
        assert_eq!(manifest.plugins["200"].time_updated, 30);
        assert!(manifest.plugins["200"].files.is_empty());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn rejects_unsafe_or_ambiguous_manifest_filenames() {
        let mut manifest = WorkshopManifest::default();
        manifest.plugins.insert(
            "123".to_owned(),
            super::PluginRecord {
                files: vec!["../outside.vpk".to_owned()],
                ..Default::default()
            },
        );
        assert!(validate_manifest(&manifest).is_err());
    }

    #[test]
    fn skips_only_items_with_matching_metadata_and_all_managed_vpks_present() {
        let root = temp_dir();
        fs::create_dir_all(&root).unwrap();
        fs::write(root.join("addon.vpk"), b"addon").unwrap();
        let plugin = PluginRecord {
            title: "Test".to_owned(),
            time_updated: 20,
            files: vec!["addon.vpk".to_owned()],
        };
        assert!(plugin_is_current(Some(&plugin), 20, &root));
        assert!(!plugin_is_current(Some(&plugin), 21, &root));
        assert!(!plugin_is_current(None, 20, &root));
        fs::remove_file(root.join("addon.vpk")).unwrap();
        assert!(!plugin_is_current(Some(&plugin), 20, &root));
        fs::remove_dir_all(root).unwrap();
    }
}
