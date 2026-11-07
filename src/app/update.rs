use crate::core::config::Config;
use crate::core::error::{AppResult, input_error};
use crate::core::language::Language;
use reqwest::blocking::Client;
use serde::Deserialize;
use std::cmp::Ordering;
use std::collections::BTreeSet;
#[cfg(unix)]
use std::ffi::OsStr;
use std::fs::{self, OpenOptions};
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicUsize, Ordering as AtomicOrdering};
use std::time::Duration;

const RELEASE_ASSET: &str = "GSBuilder-linux-x86_64.tar.gz";
const MAX_ARCHIVE_SIZE: u64 = 128 * 1024 * 1024;
const MAX_BINARY_SIZE: u64 = 128 * 1024 * 1024;
const MAX_UNPACKED_ARCHIVE_SIZE: u64 = 512 * 1024 * 1024;

#[derive(Deserialize)]
struct Release {
    tag_name: String,
    assets: Vec<ReleaseAsset>,
}

#[derive(Deserialize)]
struct ReleaseAsset {
    name: String,
    browser_download_url: String,
}

pub fn check_and_update(config: &Config, language: &Language) -> AppResult<bool> {
    if config.updates.github_repository.is_empty() {
        return Ok(false);
    }

    #[cfg(not(all(target_os = "linux", target_arch = "x86_64")))]
    {
        println!("{}", language.text("update.unsupported_platform")?);
        return Ok(false);
    }

    #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
    {
        let source_root = match source_project_root() {
            Ok(root) => root,
            Err(error) => {
                eprintln!(
                    "{}",
                    language.format("update.check_failed", &[("error", &error.to_string())])?
                );
                return Ok(false);
            }
        };
        let release = match latest_release(config, source_root.is_none()) {
            Ok(release) => release,
            Err(error) => {
                eprintln!(
                    "{}",
                    language.format("update.check_failed", &[("error", &error.to_string())])?
                );
                return Ok(false);
            }
        };
        let Some(release) = release else {
            return Ok(false);
        };
        println!(
            "{}",
            language.format(
                "update.available",
                &[("version", release.tag_name.trim_start_matches('v'))]
            )?
        );
        print!("{}", language.text("update.confirm")?);
        io::stdout().flush()?;
        let mut answer = String::new();
        if io::stdin().read_line(&mut answer)? == 0
            || !matches!(
                answer.trim().to_ascii_lowercase().as_str(),
                "y" | "yes" | "はい"
            )
        {
            return Ok(false);
        }

        let result = if let Some(project_root) = source_root {
            apply_source_update(&release, &project_root, language)
        } else {
            apply_update(&release)
        };
        match result {
            Ok(()) => {
                println!("{}", language.text("update.restart")?);
                Ok(true)
            }
            Err(error) => {
                eprintln!(
                    "{}",
                    language.format("update.install_failed", &[("error", &error.to_string())])?
                );
                Ok(false)
            }
        }
    }
}

fn latest_release(config: &Config, require_binary_asset: bool) -> AppResult<Option<Release>> {
    let client = Client::builder()
        .user_agent(&config.network.user_agent)
        .timeout(Duration::from_secs(15))
        .build()?;
    let release: Release = client
        .get(format!(
            "https://api.github.com/repos/{}/releases/latest",
            config.updates.github_repository
        ))
        .header("Accept", "application/vnd.github+json")
        .send()?
        .error_for_status()?
        .json()?;
    let current_version = env!("CARGO_PKG_VERSION");
    if compare_versions(&release.tag_name, current_version)? != Ordering::Greater {
        return Ok(None);
    }
    if require_binary_asset
        && !release.assets.iter().any(|asset| {
            asset.name == RELEASE_ASSET && is_github_release_url(&asset.browser_download_url)
        })
    {
        return Err(input_error(format!(
            "GitHub release {} is missing a valid {RELEASE_ASSET} asset",
            release.tag_name
        )));
    }
    Ok(Some(release))
}

fn source_project_root() -> AppResult<Option<PathBuf>> {
    if let Some(project_root) = std::env::var_os("GSBUILDER_PROJECT_DIR")
        && let Some(project_root) = source_project_root_at(Path::new(&project_root))
    {
        return Ok(Some(project_root));
    }

    let executable = std::env::current_exe()?;
    Ok(executable.ancestors().find_map(source_project_root_at))
}

fn source_project_root_at(path: &Path) -> Option<PathBuf> {
    is_source_project(path).then(|| path.to_path_buf())
}

fn is_source_project(path: &Path) -> bool {
    path.join("Cargo.toml").is_file() && path.join(".git").exists()
}

fn apply_update(release: &Release) -> AppResult<()> {
    let asset = release
        .assets
        .iter()
        .find(|asset| {
            asset.name == RELEASE_ASSET && is_github_release_url(&asset.browser_download_url)
        })
        .ok_or_else(|| input_error(format!("Release asset not found: {RELEASE_ASSET}")))?;
    let response = Client::builder()
        .user_agent(concat!("GSBuilder/", env!("CARGO_PKG_VERSION")))
        .timeout(Duration::from_secs(120))
        .build()?
        .get(&asset.browser_download_url)
        .send()?
        .error_for_status()?;
    if response
        .content_length()
        .is_some_and(|length| length > MAX_ARCHIVE_SIZE)
    {
        return Err(input_error(
            "Release archive exceeds the 128 MiB size limit",
        ));
    }
    let mut archive = Vec::new();
    response
        .take(MAX_ARCHIVE_SIZE + 1)
        .read_to_end(&mut archive)?;
    if archive.len() as u64 > MAX_ARCHIVE_SIZE {
        return Err(input_error(
            "Release archive exceeds the 128 MiB size limit",
        ));
    }

    let executable = std::env::current_exe()?;
    let replacement = replacement_path(&executable);
    if let Err(error) = write_binary_from_archive(&archive, &replacement) {
        let _ = fs::remove_file(&replacement);
        return Err(error);
    }
    install_replacement(&replacement, &executable)
}

fn apply_source_update(
    release: &Release,
    project_root: &Path,
    language: &Language,
) -> AppResult<()> {
    let tag_ref = format!("refs/tags/{}", release.tag_name);
    run_git(project_root, &["fetch", "--no-tags", "origin", &tag_ref])?;
    let workspace = create_update_workspace()?;
    let source = workspace.join("source");
    let result = (|| {
        add_release_worktree(project_root, &source)?;
        merge_user_configuration(project_root, &source, &release.tag_name)?;
        merge_minecraft_apis(project_root, &source)?;

        println!("{}", language.text("update.source.staging")?);
        println!("{}", language.text("update.source.building")?);
        let status = Command::new("cargo")
            .arg("build")
            .arg("--release")
            .arg("--locked")
            .arg("--manifest-path")
            .arg(source.join("Cargo.toml"))
            .arg("--target-dir")
            .arg(workspace.join("target"))
            .arg("--quiet")
            .status()?;
        if !status.success() {
            return Err(input_error(format!(
                "Could not build the updated source checkout: {status}"
            )));
        }

        let built_executable = workspace.join("target/release/GSBuilder");
        let metadata = fs::metadata(&built_executable)?;
        if !metadata.is_file() || metadata.len() == 0 || metadata.len() > MAX_BINARY_SIZE {
            return Err(input_error(format!(
                "Built executable has an invalid file size: {}",
                built_executable.display()
            )));
        }
        let executable = std::env::current_exe()?;
        let replacement = replacement_path(&executable);
        if let Err(error) = copy_executable(&built_executable, &replacement, metadata.len()) {
            if replacement.exists() {
                fs::remove_file(&replacement)?;
            }
            return Err(error);
        }
        let backups = match sync_release_files(&source, project_root, &workspace) {
            Ok(backups) => backups,
            Err(error) => {
                if let Err(cleanup_error) = fs::remove_file(&replacement) {
                    return Err(input_error(format!(
                        "{error}; additionally failed to remove prepared executable: {cleanup_error}"
                    )));
                }
                return Err(error);
            }
        };
        if let Err(error) = install_replacement(&replacement, &executable) {
            return match rollback_source_files(&backups) {
                Ok(()) => Err(error),
                Err(rollback_error) => Err(input_error(format!(
                    "{error}; failed to roll back source files after executable replacement failed: {rollback_error}"
                ))),
            };
        }
        Ok(())
    })();

    let cleanup = cleanup_update_workspace(project_root, &source, &workspace);
    match (result, cleanup) {
        (Err(error), Err(cleanup_error)) => Err(input_error(format!(
            "{error}; additionally failed to clean update staging files: {cleanup_error}"
        ))),
        (Err(error), Ok(())) => Err(error),
        (Ok(()), Err(error)) => Err(error),
        (Ok(()), Ok(())) => Ok(()),
    }
}

fn create_update_workspace() -> AppResult<PathBuf> {
    static NEXT_ID: AtomicUsize = AtomicUsize::new(0);
    for _ in 0..100 {
        let path = std::env::temp_dir().join(format!(
            "gsbuilder-update-{}-{}",
            std::process::id(),
            NEXT_ID.fetch_add(1, AtomicOrdering::Relaxed)
        ));
        #[cfg(unix)]
        let create_result = {
            use std::os::unix::fs::DirBuilderExt;
            let mut builder = fs::DirBuilder::new();
            builder.mode(0o700);
            builder.create(&path)
        };
        #[cfg(not(unix))]
        let create_result = fs::create_dir(&path);
        match create_result {
            Ok(()) => return Ok(path),
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(Box::new(error)),
        }
    }
    Err(input_error(
        "Could not allocate a unique update staging directory",
    ))
}

fn add_release_worktree(project_root: &Path, destination: &Path) -> AppResult<()> {
    let output = Command::new("git")
        .arg("-C")
        .arg(project_root)
        .args(["worktree", "add", "--detach"])
        .arg(destination)
        .arg("FETCH_HEAD")
        .output()?;
    if !output.status.success() {
        return Err(input_error(format!(
            "Could not stage release source: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        )));
    }
    Ok(())
}

fn validate_parent_directories(root: &Path, relative_path: &Path) -> AppResult<()> {
    let mut current = root.to_path_buf();
    let mut missing_parent = false;
    for component in relative_path
        .parent()
        .into_iter()
        .flat_map(Path::components)
    {
        current.push(component);
        if missing_parent {
            continue;
        }
        match fs::symlink_metadata(&current) {
            Ok(metadata) if metadata.file_type().is_dir() => {}
            Ok(_) => {
                return Err(input_error(format!(
                    "Refusing to update through a non-directory source path: {}",
                    current.display()
                )));
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => missing_parent = true,
            Err(error) => return Err(Box::new(error)),
        }
    }
    Ok(())
}

fn cleanup_update_workspace(project_root: &Path, source: &Path, workspace: &Path) -> AppResult<()> {
    if source.join(".git").exists() {
        run_git_with_path(project_root, source)?;
    }
    if workspace.exists() {
        fs::remove_dir_all(workspace)?;
    }
    Ok(())
}

fn run_git_with_path(project_root: &Path, args_path: &Path) -> AppResult<()> {
    let output = Command::new("git")
        .arg("-C")
        .arg(project_root)
        .args(["worktree", "remove", "--force"])
        .arg(args_path)
        .output()?;
    if !output.status.success() {
        return Err(input_error(format!(
            "Could not remove temporary source worktree: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        )));
    }
    Ok(())
}

fn merge_user_configuration(
    project_root: &Path,
    source: &Path,
    release_tag: &str,
) -> AppResult<()> {
    let defaults = fs::read_to_string(source.join("config.toml"))?;
    let current = fs::read_to_string(project_root.join("config.toml"))?;
    let merged = merge_config_contents(&defaults, &current, release_tag)?;
    fs::write(source.join("config.toml"), merged)?;
    Ok(())
}

fn merge_config_contents(defaults: &str, current: &str, release_tag: &str) -> AppResult<String> {
    let mut defaults = defaults
        .parse::<toml_edit::DocumentMut>()
        .map_err(|error| input_error(format!("Could not parse release config.toml: {error}")))?;
    let current = current
        .parse::<toml_edit::DocumentMut>()
        .map_err(|error| input_error(format!("Could not parse existing config.toml: {error}")))?;
    merge_config_items(defaults.as_item_mut(), current.as_item());

    let version = release_tag.trim_start_matches('v');
    let user_agent = defaults
        .get("network")
        .and_then(toml_edit::Item::as_table)
        .and_then(|network| network.get("user_agent"))
        .and_then(toml_edit::Item::as_str)
        .unwrap_or_default();
    if user_agent.starts_with("GSBuilder/") {
        defaults["network"]["user_agent"] = toml_edit::value(format!("GSBuilder/{version}"));
    }
    let merged = defaults.to_string();
    toml::from_str::<Config>(&merged)
        .map_err(|error| input_error(format!("Merged config.toml is invalid: {error}")))?;
    Ok(merged)
}

fn merge_config_items(defaults: &mut toml_edit::Item, current: &toml_edit::Item) {
    let (Some(default_table), Some(current_table)) = (defaults.as_table_mut(), current.as_table())
    else {
        *defaults = current.clone();
        return;
    };
    for (key, default_value) in default_table.iter_mut() {
        if let Some(current_value) = current_table.get(key.get()) {
            if default_value.is_table() && current_value.is_table() {
                merge_config_items(default_value, current_value);
            } else {
                *default_value = current_value.clone();
            }
        }
    }
}

fn merge_minecraft_apis(project_root: &Path, source: &Path) -> AppResult<()> {
    let current_path = project_root.join("data/minecraft/apis.json");
    let release_path = source.join("data/minecraft/apis.json");
    if !current_path.exists() || !release_path.exists() {
        return Ok(());
    }
    let current: serde_json::Value = serde_json::from_slice(&fs::read(&current_path)?)?;
    let mut defaults: serde_json::Value = serde_json::from_slice(&fs::read(&release_path)?)?;
    let current_apis = current
        .get("apis")
        .and_then(serde_json::Value::as_array)
        .ok_or_else(|| input_error("Existing Minecraft API registry has no apis array"))?;
    let default_apis = defaults
        .get_mut("apis")
        .and_then(serde_json::Value::as_array_mut)
        .ok_or_else(|| input_error("Release Minecraft API registry has no apis array"))?;
    let current_ids = current_apis
        .iter()
        .map(|api| {
            api.get("id")
                .and_then(serde_json::Value::as_str)
                .ok_or_else(|| input_error("Existing Minecraft API entry has no string id"))
        })
        .collect::<AppResult<BTreeSet<_>>>()?;
    for api in default_apis.iter() {
        if api.get("id").and_then(serde_json::Value::as_str).is_none() {
            return Err(input_error("Release Minecraft API entry has no string id"));
        }
    }
    let mut merged_apis = current_apis.clone();
    merged_apis.extend(
        default_apis
            .iter()
            .filter(|api| {
                !current_ids.contains(api.get("id").and_then(serde_json::Value::as_str).unwrap())
            })
            .cloned(),
    );
    defaults["apis"] = serde_json::Value::Array(merged_apis);
    fs::write(release_path, serde_json::to_vec_pretty(&defaults)?)?;
    Ok(())
}

fn tracked_files(root: &Path) -> AppResult<BTreeSet<PathBuf>> {
    let output = Command::new("git")
        .arg("-C")
        .arg(root)
        .args(["ls-files", "-z"])
        .output()?;
    if !output.status.success() {
        return Err(input_error(format!(
            "Could not list tracked source files: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        )));
    }
    #[cfg(unix)]
    use std::os::unix::ffi::OsStrExt;
    let mut files = BTreeSet::new();
    for path in output
        .stdout
        .split(|byte| *byte == 0)
        .filter(|path| !path.is_empty())
    {
        #[cfg(unix)]
        let path = PathBuf::from(OsStr::from_bytes(path));
        #[cfg(not(unix))]
        let path = PathBuf::from(std::str::from_utf8(path)?);
        if path.is_absolute()
            || path
                .components()
                .any(|component| !matches!(component, std::path::Component::Normal(_)))
        {
            return Err(input_error("Git returned an unsafe tracked file path"));
        }
        files.insert(path);
    }
    Ok(files)
}

fn sync_release_files(
    source: &Path,
    project_root: &Path,
    workspace: &Path,
) -> AppResult<Vec<(PathBuf, Option<PathBuf>)>> {
    let source = source.canonicalize()?;
    let project_root = project_root.canonicalize()?;
    let release_files = tracked_files(&source)?;
    let current_files = tracked_files(&project_root)?;
    let mut changes = Vec::new();

    for path in &release_files {
        let source_path = source.join(path);
        validate_parent_directories(&source, path)?;
        let metadata = fs::symlink_metadata(&source_path)?;
        if !metadata.file_type().is_file() {
            return Err(input_error(format!(
                "Release source contains a non-regular file: {}",
                source_path.display()
            )));
        }
        changes.push((path.clone(), Some(source_path)));
    }
    for path in current_files.difference(&release_files) {
        changes.push((path.clone(), None));
    }

    let backup_root = workspace.join("backups");
    fs::create_dir(&backup_root)?;
    let mut backups = Vec::new();
    for (index, (path, _)) in changes.iter().enumerate() {
        let target = project_root.join(path);
        validate_parent_directories(&project_root, path)?;
        let backup = match fs::symlink_metadata(&target) {
            Ok(metadata) if metadata.file_type().is_file() => {
                let backup = backup_root.join(index.to_string());
                fs::copy(&target, &backup)?;
                fs::set_permissions(&backup, metadata.permissions())?;
                Some(backup)
            }
            Ok(_) => {
                return Err(input_error(format!(
                    "Refusing to overwrite a non-regular source path: {}",
                    target.display()
                )));
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => None,
            Err(error) => return Err(Box::new(error)),
        };
        backups.push((target, backup));
    }

    let mut applied = 0;
    let sync_result = (|| {
        for ((_, source_path), (target, _)) in changes.iter().zip(&backups) {
            if let Some(source_path) = source_path {
                copy_file_atomically(source_path, target, applied)?;
            } else if target.exists() {
                fs::remove_file(target)?;
            }
            applied += 1;
        }
        Ok::<_, Box<dyn std::error::Error>>(())
    })();
    if let Err(error) = sync_result {
        let rollback = rollback_source_files(&backups[..applied]);
        return match rollback {
            Ok(()) => Err(error),
            Err(rollback_error) => Err(input_error(format!(
                "{error}; failed to roll back partial source update: {rollback_error}"
            ))),
        };
    }
    Ok(backups)
}

fn copy_file_atomically(source: &Path, destination: &Path, index: usize) -> AppResult<()> {
    let parent = destination
        .parent()
        .ok_or_else(|| input_error("Updated source file has no parent directory"))?;
    fs::create_dir_all(parent)?;
    let file_name = destination
        .file_name()
        .unwrap_or_default()
        .to_string_lossy();
    let temporary = parent.join(format!(
        ".{file_name}.gsbuilder-update-{}-{index}.tmp",
        std::process::id()
    ));
    let result = (|| {
        let mut input = fs::File::open(source)?;
        let mut output = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temporary)?;
        io::copy(&mut input, &mut output)?;
        output.sync_all()?;
        fs::set_permissions(&temporary, fs::metadata(source)?.permissions())?;
        fs::rename(&temporary, destination)?;
        Ok::<_, io::Error>(())
    })();
    if let Err(error) = result {
        if temporary.exists() {
            fs::remove_file(temporary)?;
        }
        return Err(Box::new(error));
    }
    Ok(())
}

fn rollback_source_files(backups: &[(PathBuf, Option<PathBuf>)]) -> AppResult<()> {
    for (index, (target, backup)) in backups.iter().enumerate().rev() {
        if let Some(backup) = backup {
            copy_file_atomically(backup, target, usize::MAX - index)?;
        } else if target.exists() {
            fs::remove_file(target)?;
        }
    }
    Ok(())
}

fn copy_executable(source: &Path, destination: &Path, expected_size: u64) -> AppResult<()> {
    let mut output = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(destination)?;
    let mut input = fs::File::open(source)?;
    let copied = io::copy(&mut input, &mut output)?;
    if copied != expected_size {
        return Err(input_error(
            "Built executable size changed while preparing the update",
        ));
    }
    output.sync_all()?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        output.set_permissions(fs::Permissions::from_mode(0o755))?;
    }
    Ok(())
}

fn run_git(project_root: &Path, args: &[&str]) -> AppResult<()> {
    let output = Command::new("git")
        .arg("-C")
        .arg(project_root)
        .args(args)
        .output()?;
    if !output.status.success() {
        return Err(input_error(format!(
            "Git command failed ({}): {}",
            args.join(" "),
            String::from_utf8_lossy(&output.stderr).trim()
        )));
    }
    Ok(())
}

fn install_replacement(replacement: &Path, executable: &Path) -> AppResult<()> {
    if let Err(error) = fs::rename(replacement, executable) {
        let _ = fs::remove_file(replacement);
        if error.kind() == io::ErrorKind::PermissionDenied {
            return Err(input_error(format!(
                "Cannot replace {}. Install GSBuilder in a writable directory or use sudo",
                executable.display()
            )));
        }
        return Err(Box::new(error));
    }
    Ok(())
}

fn write_binary_from_archive(archive_bytes: &[u8], destination: &Path) -> AppResult<()> {
    let decoder = flate2::read::GzDecoder::new(archive_bytes);
    let mut archive = tar::Archive::new(decoder);
    let entries = archive.entries()?;
    let mut total_size = 0_u64;
    let mut binary = None;
    for entry in entries {
        let mut entry = entry?;
        let size = entry.size();
        total_size = total_size
            .checked_add(size)
            .ok_or_else(|| input_error("Release archive size overflow"))?;
        if total_size > MAX_UNPACKED_ARCHIVE_SIZE {
            return Err(input_error(
                "Unpacked release archive exceeds the 512 MiB size limit",
            ));
        }
        if entry.path()? != Path::new("GSBuilder") {
            continue;
        }
        if !entry.header().entry_type().is_file() {
            return Err(input_error("GSBuilder archive entry is not a regular file"));
        }
        if size == 0 || size > MAX_BINARY_SIZE {
            return Err(input_error("Updated executable has an invalid file size"));
        }
        if binary.is_some() {
            return Err(input_error(
                "Release archive contains more than one GSBuilder executable",
            ));
        }
        let mut contents = Vec::with_capacity(size as usize);
        let copied = io::copy(&mut entry, &mut contents)?;
        if copied != size {
            return Err(input_error(
                "Updated executable size does not match its archive entry",
            ));
        }
        binary = Some(contents);
    }
    let binary = binary.ok_or_else(|| {
        input_error("Release archive does not contain a root-level GSBuilder executable")
    })?;
    let mut output = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(destination)?;
    output.write_all(&binary)?;
    output.sync_all()?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        output.set_permissions(fs::Permissions::from_mode(0o755))?;
    }
    Ok(())
}

fn replacement_path(executable: &Path) -> PathBuf {
    let file_name = executable.file_name().unwrap_or_default().to_string_lossy();
    executable.with_file_name(format!(".{file_name}.update-{}.tmp", std::process::id()))
}

fn is_github_release_url(value: &str) -> bool {
    reqwest::Url::parse(value).is_ok_and(|url| {
        url.scheme() == "https"
            && url.host_str() == Some("github.com")
            && url.path().contains("/releases/download/")
    })
}

fn compare_versions(remote: &str, current: &str) -> AppResult<Ordering> {
    let remote = parse_version(remote)?;
    let current = parse_version(current)?;
    Ok(remote.cmp(&current))
}

#[derive(Debug, Eq, PartialEq)]
struct Version {
    major: u64,
    minor: u64,
    patch: u64,
    prerelease: Option<String>,
}

impl Ord for Version {
    fn cmp(&self, other: &Self) -> Ordering {
        (self.major, self.minor, self.patch)
            .cmp(&(other.major, other.minor, other.patch))
            .then_with(|| match (&self.prerelease, &other.prerelease) {
                (None, None) => Ordering::Equal,
                (None, Some(_)) => Ordering::Greater,
                (Some(_), None) => Ordering::Less,
                (Some(left), Some(right)) => left.cmp(right),
            })
    }
}

impl PartialOrd for Version {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

fn parse_version(value: &str) -> AppResult<Version> {
    let value = value.strip_prefix('v').unwrap_or(value);
    let value = value.split_once('+').map_or(value, |(version, _)| version);
    let (release, prerelease) = value
        .split_once('-')
        .map_or((value, None), |(release, prerelease)| {
            (release, Some(prerelease.to_owned()))
        });
    let mut parts = release.split('.');
    let parse_part = |part: Option<&str>| {
        part.and_then(|part| part.parse::<u64>().ok())
            .ok_or_else(|| input_error(format!("Invalid version tag: {value}")))
    };
    let version = Version {
        major: parse_part(parts.next())?,
        minor: parse_part(parts.next())?,
        patch: parse_part(parts.next())?,
        prerelease,
    };
    if parts.next().is_some() {
        return Err(input_error(format!("Invalid version tag: {value}")));
    }
    Ok(version)
}

#[cfg(test)]
mod tests {
    use super::{
        check_and_update, compare_versions, create_update_workspace, is_github_release_url,
        merge_config_contents, merge_minecraft_apis, replacement_path, source_project_root_at,
        sync_release_files, write_binary_from_archive,
    };
    use crate::core::config::Config;
    use crate::core::language::Language;
    use flate2::Compression;
    use flate2::write::GzEncoder;
    use std::cmp::Ordering;
    use std::fs;
    use std::io::Cursor;
    use std::path::{Path, PathBuf};
    use std::process::Command;
    use std::sync::atomic::{AtomicUsize, Ordering as AtomicOrdering};

    fn temp_dir() -> PathBuf {
        static NEXT_ID: AtomicUsize = AtomicUsize::new(0);
        std::env::temp_dir().join(format!(
            "gsbuilder-update-{}-{}",
            std::process::id(),
            NEXT_ID.fetch_add(1, AtomicOrdering::Relaxed)
        ))
    }

    #[test]
    fn compares_release_versions_and_prereleases() {
        assert_eq!(
            compare_versions("v1.2.0", "1.1.9").unwrap(),
            Ordering::Greater
        );
        assert_eq!(compare_versions("1.2.0", "1.2.0").unwrap(), Ordering::Equal);
        assert_eq!(
            compare_versions("1.2.0", "1.2.0-rc.1").unwrap(),
            Ordering::Greater
        );
        assert!(compare_versions("latest", "1.0.0").is_err());
    }

    #[test]
    fn skips_update_checks_when_no_repository_is_configured() {
        let config = Config::default();
        let language = Language::load(&config.language_file("en").unwrap()).unwrap();
        assert!(!check_and_update(&config, &language).unwrap());
    }

    #[test]
    fn accepts_only_https_github_release_asset_urls() {
        assert!(is_github_release_url(
            "https://github.com/owner/repo/releases/download/v1.0.1/GSBuilder-linux-x86_64.tar.gz"
        ));
        assert!(!is_github_release_url(
            "http://github.com/owner/repo/releases/download/v1.0.1/file"
        ));
        assert!(!is_github_release_url(
            "https://example.com/releases/download/v1.0.1/file"
        ));
    }

    #[test]
    fn creates_a_temporary_path_next_to_the_executable() {
        assert_eq!(
            replacement_path(Path::new("/opt/gsbuilder/GSBuilder")),
            PathBuf::from(format!(
                "/opt/gsbuilder/.GSBuilder.update-{}.tmp",
                std::process::id()
            ))
        );
    }

    #[test]
    fn creates_a_private_update_workspace() {
        let workspace = create_update_workspace().unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                fs::metadata(&workspace).unwrap().permissions().mode() & 0o777,
                0o700
            );
        }
        fs::remove_dir_all(workspace).unwrap();
    }

    #[test]
    fn packaged_release_directory_is_not_mistaken_for_a_source_checkout() {
        let directory = temp_dir();
        fs::create_dir_all(&directory).unwrap();
        fs::write(directory.join("Cargo.toml"), "[package]\n").unwrap();
        assert!(source_project_root_at(&directory).is_none());
        fs::create_dir(directory.join(".git")).unwrap();
        assert_eq!(source_project_root_at(&directory), Some(directory.clone()));
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn config_update_preserves_user_values_and_adds_defaults() {
        let defaults = r#"
[language]
code = "ja"

[steam]
steamcmd_executable = "steamcmd"
login_mode = "anonymous"
run_as_user = ""

[network]
timeout_seconds = 120
user_agent = "GSBuilder/2.0.0"
"#;
        let current = r#"
[language]
code = "en"

[steam]
steamcmd_executable = "/custom/steamcmd"
login_mode = "account"
run_as_user = ""
removed_setting = true

[network]
timeout_seconds = 30
user_agent = "GSBuilder/1.5.0"
"#;
        let merged = merge_config_contents(defaults, current, "v2.0.0").unwrap();
        let merged: toml::Value = toml::from_str(&merged).unwrap();
        assert_eq!(merged["language"]["code"].as_str(), Some("en"));
        assert_eq!(merged["steam"]["run_as_user"].as_str(), Some(""));
        assert_eq!(
            merged["steam"]["steamcmd_executable"].as_str(),
            Some("/custom/steamcmd")
        );
        assert_eq!(merged["steam"].get("removed_setting"), None);
        assert_eq!(merged["network"]["timeout_seconds"].as_integer(), Some(30));
        assert_eq!(
            merged["network"]["user_agent"].as_str(),
            Some("GSBuilder/2.0.0")
        );

        let custom_agent = merge_config_contents(
            defaults,
            &current.replace("GSBuilder/1.5.0", "custom-client/7"),
            "v2.0.0",
        )
        .unwrap();
        let custom_agent: toml::Value = toml::from_str(&custom_agent).unwrap();
        assert_eq!(
            custom_agent["network"]["user_agent"].as_str(),
            Some("custom-client/7")
        );
    }

    #[test]
    fn api_registry_update_keeps_custom_entries_and_adds_new_defaults() {
        let directory = temp_dir();
        let current = directory.join("data/minecraft/apis.json");
        let release = directory.join("release/data/minecraft/apis.json");
        fs::create_dir_all(current.parent().unwrap()).unwrap();
        fs::create_dir_all(release.parent().unwrap()).unwrap();
        fs::write(
            &current,
            r#"{"apis":[{"id":"mojang","name":"My Mojang","manifest_url":"https://custom.invalid"},{"id":"custom","name":"Custom"}]}"#,
        )
        .unwrap();
        fs::write(
            &release,
            r#"{"apis":[{"id":"mojang","name":"Mojang","manifest_url":"https://official.invalid"},{"id":"fabric","name":"Fabric"}]}"#,
        )
        .unwrap();

        merge_minecraft_apis(&directory, &directory.join("release")).unwrap();
        let merged: serde_json::Value =
            serde_json::from_slice(&fs::read(release).unwrap()).unwrap();
        let apis = merged["apis"].as_array().unwrap();
        assert_eq!(apis.len(), 3);
        assert_eq!(apis[0]["name"], "My Mojang");
        assert_eq!(apis[1]["id"], "custom");
        assert_eq!(apis[2]["id"], "fabric");
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn source_sync_overwrites_tracked_files_removes_obsolete_and_keeps_untracked() {
        let directory = temp_dir();
        let current = directory.join("current");
        let release = directory.join("release");
        let workspace = directory.join("workspace");
        for root in [&current, &release] {
            fs::create_dir_all(root).unwrap();
            assert!(
                Command::new("git")
                    .arg("-C")
                    .arg(root)
                    .args(["init", "-q"])
                    .status()
                    .unwrap()
                    .success()
            );
        }
        fs::write(current.join("source.rs"), "local edit").unwrap();
        fs::write(current.join("obsolete.rs"), "old").unwrap();
        fs::write(current.join("user-file.txt"), "keep").unwrap();
        stage_test_files(&current, &["source.rs", "obsolete.rs"]);

        fs::write(release.join("source.rs"), "new release").unwrap();
        fs::write(release.join("added.rs"), "new file").unwrap();
        stage_test_files(&release, &["source.rs", "added.rs"]);
        fs::create_dir_all(&workspace).unwrap();

        sync_release_files(&release, &current, &workspace).unwrap();
        assert_eq!(
            fs::read_to_string(current.join("source.rs")).unwrap(),
            "new release"
        );
        assert_eq!(
            fs::read_to_string(current.join("added.rs")).unwrap(),
            "new file"
        );
        assert!(!current.join("obsolete.rs").exists());
        assert_eq!(
            fs::read_to_string(current.join("user-file.txt")).unwrap(),
            "keep"
        );
        fs::remove_dir_all(directory).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn source_sync_refuses_to_write_through_symlinked_directories() {
        let directory = temp_dir();
        let current = directory.join("current");
        let release = directory.join("release");
        let workspace = directory.join("workspace");
        let outside = directory.join("outside");
        for root in [&current, &release] {
            fs::create_dir_all(root).unwrap();
            assert!(
                Command::new("git")
                    .arg("-C")
                    .arg(root)
                    .args(["init", "-q"])
                    .status()
                    .unwrap()
                    .success()
            );
        }
        fs::create_dir_all(current.join("nested")).unwrap();
        fs::write(current.join("nested/source.rs"), "local edit").unwrap();
        stage_test_files(&current, &["nested/source.rs"]);
        fs::create_dir_all(release.join("nested")).unwrap();
        fs::write(release.join("nested/source.rs"), "new release").unwrap();
        stage_test_files(&release, &["nested/source.rs"]);

        fs::create_dir_all(&outside).unwrap();
        fs::write(outside.join("source.rs"), "outside file").unwrap();
        fs::remove_dir_all(current.join("nested")).unwrap();
        std::os::unix::fs::symlink(&outside, current.join("nested")).unwrap();
        fs::create_dir_all(&workspace).unwrap();

        assert!(sync_release_files(&release, &current, &workspace).is_err());
        assert_eq!(
            fs::read_to_string(outside.join("source.rs")).unwrap(),
            "outside file"
        );
        fs::remove_dir_all(directory).unwrap();
    }

    fn stage_test_files(root: &Path, files: &[&str]) {
        assert!(
            Command::new("git")
                .arg("-C")
                .arg(root)
                .arg("add")
                .args(files)
                .status()
                .unwrap()
                .success()
        );
    }

    #[test]
    fn extracts_only_the_root_executable_from_a_release_archive() {
        let directory =
            std::env::temp_dir().join(format!("gsbuilder-update-test-{}", std::process::id()));
        fs::create_dir_all(&directory).unwrap();
        let destination = directory.join("GSBuilder.new");
        let mut archive_bytes = Vec::new();
        {
            let encoder = GzEncoder::new(&mut archive_bytes, Compression::default());
            let mut archive = tar::Builder::new(encoder);
            let mut header = tar::Header::new_gnu();
            header.set_size(6);
            header.set_mode(0o755);
            header.set_cksum();
            archive
                .append_data(&mut header, "GSBuilder", Cursor::new(b"binary"))
                .unwrap();
            archive.into_inner().unwrap().finish().unwrap();
        }

        write_binary_from_archive(&archive_bytes, &destination).unwrap();
        assert_eq!(fs::read(&destination).unwrap(), b"binary");
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn rejects_archives_without_a_root_executable() {
        let mut archive_bytes = Vec::new();
        {
            let encoder = GzEncoder::new(&mut archive_bytes, Compression::default());
            let mut archive = tar::Builder::new(encoder);
            let mut header = tar::Header::new_gnu();
            header.set_size(6);
            header.set_mode(0o644);
            header.set_cksum();
            archive
                .append_data(&mut header, "nested/GSBuilder", Cursor::new(b"binary"))
                .unwrap();
            archive.into_inner().unwrap().finish().unwrap();
        }
        assert!(
            write_binary_from_archive(&archive_bytes, Path::new("/tmp/gsbuilder-invalid-update"))
                .is_err()
        );
    }
}
