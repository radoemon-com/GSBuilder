use crate::core::config::Config;
use crate::core::error::{AppResult, input_error};
use crate::core::files::copy_path;
use crate::core::language::Language;
use crate::core::paths::is_valid_component;
use serde::Deserialize;
use sha1::Sha1;
use sha2::{Digest, Sha512};
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write};
use std::path::Path;
use std::sync::atomic::{AtomicUsize, Ordering};
use zip::ZipArchive;

const MAX_MOD_SIZE: u64 = 512 * 1024 * 1024;
const MAX_PACK_SIZE: u64 = 4 * 1024 * 1024 * 1024;
const MAX_ARCHIVE_ENTRIES: usize = 20_000;

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ModrinthPack {
    format_version: u32,
    game: String,
    files: Vec<ModrinthFile>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ModrinthFile {
    path: String,
    #[serde(default)]
    hashes: ModHashes,
    downloads: Vec<String>,
    #[serde(default)]
    file_size: Option<u64>,
    #[serde(default)]
    env: ModEnvironment,
}

#[derive(Default, Deserialize)]
struct ModHashes {
    #[serde(default)]
    sha1: Option<String>,
    #[serde(default)]
    sha512: Option<String>,
}

#[derive(Default, Deserialize)]
struct ModEnvironment {
    #[serde(default)]
    server: Option<String>,
}

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
    if fs::symlink_metadata(source)?.file_type().is_symlink() {
        return Err(input_error(format!(
            "Symbolic links are not supported as mod sources: {}",
            source.display()
        )));
    }
    let server = config.minecraft_servers_path().join(server_name);
    let server_metadata = match fs::symlink_metadata(&server) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Err(input_error(format!(
                "Minecraft server does not exist: {}",
                server.display()
            )));
        }
        Err(error) => return Err(Box::new(error)),
    };
    if !server_metadata.file_type().is_dir() {
        return Err(input_error(format!(
            "Minecraft server path is not a real directory: {}",
            server.display()
        )));
    }
    let destination = server.join("mods");
    if source.is_file()
        && source
            .extension()
            .is_some_and(|extension| extension.eq_ignore_ascii_case("zip"))
    {
        let count = import_modpack(config, source, &destination)?;
        println!(
            "{}",
            language.format(
                "cli.mods.imported",
                &[
                    ("count", &count.to_string()),
                    ("path", &destination.display().to_string()),
                ],
            )?
        );
        return Ok(());
    }
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

fn import_modpack(config: &Config, archive_path: &Path, destination: &Path) -> AppResult<usize> {
    let file = File::open(archive_path)?;
    let mut archive = ZipArchive::new(file)?;
    if archive.len() > MAX_ARCHIVE_ENTRIES {
        return Err(input_error("Modpack ZIP contains too many entries"));
    }

    let stage = temporary_sibling(destination, "import");
    let backup = temporary_sibling(destination, "backup");
    fs::create_dir(&stage)?;

    let result = (|| {
        let is_curseforge_pack = archive.by_name("manifest.json").is_ok();
        let count = if let Ok(index) = archive.by_name("modrinth.index.json") {
            if index.size() > 16 * 1024 * 1024 {
                return Err(input_error("Modrinth index file is too large"));
            }
            let pack: ModrinthPack = serde_json::from_reader(index)?;
            download_modrinth_mods(config, &pack, &stage)?
        } else if archive.by_name("manifest.json").is_ok() {
            import_bundled_mods(&mut archive, &stage)?
        } else {
            import_bundled_mods(&mut archive, &stage)?
        };
        if count == 0 {
            if is_curseforge_pack {
                return Err(input_error(
                    "CurseForge modpack manifest references remote file IDs; this ZIP must include the mod JARs in its overrides/mods folder",
                ));
            }
            return Err(input_error("The ZIP contains no server mods"));
        }
        replace_mod_directory(destination, &stage, &backup)?;
        Ok(count)
    })();

    if stage.exists() {
        fs::remove_dir_all(&stage)?;
    }
    if backup.exists() {
        fs::remove_dir_all(&backup)?;
    }
    result
}

fn download_modrinth_mods(
    config: &Config,
    pack: &ModrinthPack,
    destination: &Path,
) -> AppResult<usize> {
    if pack.format_version != 1 || pack.game != "minecraft" {
        return Err(input_error(
            "Unsupported Modrinth pack format or game; expected Minecraft format version 1",
        ));
    }
    let client = crate::core::http::http_client(config)?;
    let mut count = 0;
    let mut total_size = 0u64;
    for file in &pack.files {
        if file.env.server.as_deref() == Some("unsupported") {
            continue;
        }
        let Some(relative_path) = mod_relative_path(&file.path) else {
            continue;
        };
        if file.file_size.is_some_and(|size| size > MAX_MOD_SIZE) {
            return Err(input_error(format!(
                "Mod file exceeds the maximum supported size: {}",
                file.path
            )));
        }

        let mut last_error = None;
        for url in &file.downloads {
            let parsed_url = reqwest::Url::parse(url)
                .map_err(|error| input_error(format!("Invalid mod download URL: {error}")))?;
            if parsed_url.scheme() != "https" {
                return Err(input_error(format!(
                    "Mod download URL must use HTTPS: {}",
                    file.path
                )));
            }
            match client.get(parsed_url).send() {
                Ok(response) if response.status().is_success() => {
                    if response.url().scheme() != "https" {
                        return Err(input_error(format!(
                            "Mod download redirected to a non-HTTPS URL: {}",
                            file.path
                        )));
                    }
                    if response
                        .content_length()
                        .is_some_and(|size| size > MAX_MOD_SIZE)
                    {
                        return Err(input_error(format!(
                            "Mod file exceeds the maximum supported size: {}",
                            file.path
                        )));
                    }
                    let mut contents = Vec::new();
                    response.take(MAX_MOD_SIZE + 1).read_to_end(&mut contents)?;
                    if contents.len() as u64 > MAX_MOD_SIZE
                        || file
                            .file_size
                            .is_some_and(|size| size != contents.len() as u64)
                    {
                        last_error = Some(format!("Unexpected size for {}", file.path));
                        continue;
                    }
                    if !hashes_match(&file.hashes, &contents)? {
                        last_error = Some(format!("Checksum mismatch for {}", file.path));
                        continue;
                    }
                    total_size = total_size.saturating_add(contents.len() as u64);
                    if total_size > MAX_PACK_SIZE {
                        return Err(input_error("Modpack exceeds the maximum supported size"));
                    }
                    let output = destination.join(&relative_path);
                    if let Some(parent) = output.parent() {
                        fs::create_dir_all(parent)?;
                    }
                    let mut output_file = OpenOptions::new()
                        .write(true)
                        .create_new(true)
                        .open(output)?;
                    output_file.write_all(&contents)?;
                    count += 1;
                    last_error = None;
                    break;
                }
                Ok(response) => {
                    last_error = Some(format!(
                        "Download failed for {}: HTTP {}",
                        file.path,
                        response.status()
                    ));
                }
                Err(error) => {
                    last_error = Some(format!("Download failed for {}: {error}", file.path));
                }
            }
        }
        if let Some(error) = last_error {
            return Err(input_error(error));
        }
        if file.downloads.is_empty() {
            return Err(input_error(format!(
                "No download URL is available for {}",
                file.path
            )));
        }
    }
    Ok(count)
}

fn import_bundled_mods(archive: &mut ZipArchive<File>, destination: &Path) -> AppResult<usize> {
    let mut count = 0;
    let mut total_size = 0;
    for index in 0..archive.len() {
        let entry = archive.by_index(index)?;
        let Some(relative_path) = bundled_mod_relative_path(entry.name()) else {
            continue;
        };
        if entry.size() > MAX_MOD_SIZE {
            return Err(input_error(format!(
                "Mod file exceeds the maximum supported size: {}",
                entry.name()
            )));
        }
        total_size += entry.size();
        if total_size > MAX_PACK_SIZE {
            return Err(input_error("Modpack exceeds the maximum supported size"));
        }
        let output = destination.join(relative_path);
        if let Some(parent) = output.parent() {
            fs::create_dir_all(parent)?;
        }
        let mut output_file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(output)?;
        let copied = std::io::copy(&mut entry.take(MAX_MOD_SIZE + 1), &mut output_file)?;
        if copied > MAX_MOD_SIZE {
            return Err(input_error("Mod file exceeds the maximum supported size"));
        }
        output_file.flush()?;
        count += 1;
    }
    Ok(count)
}

fn mod_relative_path(path: &str) -> Option<std::path::PathBuf> {
    let Some(relative_path) = path.strip_prefix("mods/") else {
        return None;
    };
    if relative_path.is_empty()
        || relative_path.contains('\\')
        || relative_path.split('/').any(|component| {
            component.is_empty()
                || matches!(component, "." | "..")
                || component.contains(':')
                || component.chars().any(char::is_control)
        })
    {
        return None;
    }
    let relative_path = Path::new(relative_path).to_path_buf();
    let file_name = relative_path.file_name()?;
    if !file_name
        .to_string_lossy()
        .to_ascii_lowercase()
        .ends_with(".jar")
    {
        return None;
    }
    Some(relative_path)
}

fn bundled_mod_relative_path(path: &str) -> Option<std::path::PathBuf> {
    let normalized = path.replace('\\', "/");
    for prefix in [
        "mods/",
        ".minecraft/mods/",
        "minecraft/mods/",
        "overrides/mods/",
        "overrides/.minecraft/mods/",
    ] {
        if let Some(relative_path) = normalized.strip_prefix(prefix) {
            return mod_relative_path(&format!("mods/{relative_path}"));
        }
    }
    None
}

fn hashes_match(hashes: &ModHashes, contents: &[u8]) -> AppResult<bool> {
    if hashes.sha1.is_none() && hashes.sha512.is_none() {
        return Err(input_error(
            "Modrinth index is missing a SHA-1 or SHA-512 checksum",
        ));
    }
    let sha1_matches = hashes.sha1.as_ref().is_none_or(|expected| {
        format!("{:x}", Sha1::digest(contents)).eq_ignore_ascii_case(expected)
    });
    let sha512_matches = hashes.sha512.as_ref().is_none_or(|expected| {
        format!("{:x}", Sha512::digest(contents)).eq_ignore_ascii_case(expected)
    });
    Ok(sha1_matches && sha512_matches)
}

fn replace_mod_directory(destination: &Path, stage: &Path, backup: &Path) -> AppResult<()> {
    if destination.exists() {
        let metadata = fs::symlink_metadata(destination)?;
        if !metadata.file_type().is_dir() {
            return Err(input_error(format!(
                "Mods path is not a directory: {}",
                destination.display()
            )));
        }
        fs::rename(destination, backup)?;
        if let Err(error) = fs::rename(stage, destination) {
            fs::rename(backup, destination)?;
            return Err(error.into());
        }
    } else {
        fs::rename(stage, destination)?;
    }
    Ok(())
}

fn temporary_sibling(path: &Path, purpose: &str) -> std::path::PathBuf {
    static NEXT_ID: AtomicUsize = AtomicUsize::new(0);
    path.with_file_name(format!(
        ".mods-{purpose}-{}-{}",
        std::process::id(),
        NEXT_ID.fetch_add(1, Ordering::Relaxed)
    ))
}

#[cfg(test)]
mod tests {
    use super::{ModrinthPack, bundled_mod_relative_path, mod_relative_path};
    use crate::core::config::Config;
    use std::fs::{self, File};
    use std::io::Write;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use zip::ZipWriter;
    use zip::write::SimpleFileOptions;

    fn test_directory() -> PathBuf {
        static NEXT_ID: AtomicUsize = AtomicUsize::new(0);
        std::env::temp_dir().join(format!(
            "gsbuilder-modpack-test-{}-{}",
            std::process::id(),
            NEXT_ID.fetch_add(1, Ordering::Relaxed)
        ))
    }

    #[test]
    fn accepts_only_safe_jar_paths_inside_mods() {
        assert_eq!(
            mod_relative_path("mods/example/Example.jar").unwrap(),
            PathBuf::from("example/Example.jar")
        );
        for path in [
            "../mods/Example.jar",
            "mods/../Example.jar",
            "mods\\Example.jar",
            "mods/config.json",
            "config/Example.jar",
        ] {
            assert!(mod_relative_path(path).is_none(), "accepted {path}");
        }
        assert_eq!(
            bundled_mod_relative_path("overrides/.minecraft/mods/example.jar").unwrap(),
            PathBuf::from("example.jar")
        );
    }

    #[test]
    fn parses_modrinth_index_field_names() {
        let pack: ModrinthPack = serde_json::from_str(
            r#"{"formatVersion":1,"game":"minecraft","versionId":"test","name":"Test","files":[],"dependencies":{}}"#,
        )
        .unwrap();
        assert_eq!(pack.format_version, 1);
        assert_eq!(pack.game, "minecraft");
    }

    #[test]
    fn replaces_mods_with_bundled_zip_mods_and_preserves_old_mods_on_failure() {
        let root = test_directory();
        let source = root.join("pack.zip");
        let destination = root.join("server/mods");
        fs::create_dir_all(&destination).unwrap();
        fs::write(destination.join("old.jar"), b"old").unwrap();

        let file = File::create(&source).unwrap();
        let mut zip = ZipWriter::new(file);
        zip.start_file("overrides/mods/new.jar", SimpleFileOptions::default())
            .unwrap();
        zip.write_all(b"new").unwrap();
        zip.start_file("overrides/config/options.txt", SimpleFileOptions::default())
            .unwrap();
        zip.write_all(b"not a mod").unwrap();
        zip.finish().unwrap();

        assert_eq!(
            super::import_modpack(&Config::default(), &source, &destination).unwrap(),
            1
        );
        assert!(!destination.join("old.jar").exists());
        assert_eq!(fs::read(destination.join("new.jar")).unwrap(), b"new");

        let invalid = root.join("empty.zip");
        let file = File::create(&invalid).unwrap();
        let mut zip = ZipWriter::new(file);
        zip.start_file("manifest.json", SimpleFileOptions::default())
            .unwrap();
        zip.write_all(br#"{"files":[]}"#).unwrap();
        zip.finish().unwrap();
        assert!(super::import_modpack(&Config::default(), &invalid, &destination).is_err());
        assert_eq!(fs::read(destination.join("new.jar")).unwrap(), b"new");
        fs::remove_dir_all(root).unwrap();
    }
}
