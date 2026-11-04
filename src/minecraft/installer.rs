use crate::core::config::Config;
use crate::core::error::{AppResult, input_error};
use crate::core::http::http_client;
use crate::core::language::Language;
use crate::minecraft::api::{MinecraftApi, validate_https_url};
use serde_json::Value;
use std::fs;
use std::path::Path;
use std::process::Command;

#[derive(Clone, Copy)]
pub enum ServerKind {
    Vanilla,
    Fabric,
    Forge,
    NeoForge,
    Paper,
}

impl ServerKind {
    pub fn parse(value: &str) -> AppResult<Self> {
        match value {
            "vanilla" => Ok(Self::Vanilla),
            "fabric" => Ok(Self::Fabric),
            "forge" => Ok(Self::Forge),
            "neoforge" => Ok(Self::NeoForge),
            "paper" => Ok(Self::Paper),
            _ => Err(input_error(
                "Minecraft type must be vanilla, fabric, forge, neoforge, or paper",
            )),
        }
    }

    pub fn provider_type(self) -> &'static str {
        match self {
            Self::Vanilla => "vanilla",
            Self::Fabric => "fabric",
            Self::Forge => "forge",
            Self::NeoForge => "neoforge",
            Self::Paper => "paper",
        }
    }

    pub fn default_api_id(self) -> &'static str {
        match self {
            Self::Vanilla => "mojang",
            _ => self.provider_type(),
        }
    }

    fn jar_name(self) -> Option<&'static str> {
        match self {
            Self::Vanilla => Some("server.jar"),
            Self::Fabric => Some("fabric-server-launch.jar"),
            Self::Forge | Self::NeoForge => None,
            Self::Paper => Some("paper-server.jar"),
        }
    }
}

pub struct InstallOutput {
    pub jar_name: Option<String>,
    pub uses_generated_script: bool,
}

pub fn install(
    config: &Config,
    target: &Path,
    version: &str,
    kind: ServerKind,
    api: &MinecraftApi,
    language: &Language,
) -> AppResult<InstallOutput> {
    match kind {
        ServerKind::Vanilla => install_vanilla(config, target, version, api)?,
        ServerKind::Fabric => install_fabric(config, target, version, api, language)?,
        ServerKind::Forge => install_forge(config, target, version, api, language)?,
        ServerKind::NeoForge => install_neoforge(config, target, version, api, language)?,
        ServerKind::Paper => install_paper(config, target, version, api)?,
    }
    Ok(InstallOutput {
        jar_name: kind.jar_name().map(str::to_owned),
        uses_generated_script: matches!(kind, ServerKind::Forge | ServerKind::NeoForge),
    })
}

fn install_vanilla(
    config: &Config,
    target: &Path,
    version: &str,
    api: &MinecraftApi,
) -> AppResult<()> {
    let manifest_url = api
        .manifest_url
        .as_deref()
        .ok_or_else(|| input_error(format!("API {} is not a vanilla manifest API", api.id)))?;
    let client = http_client(config)?;
    let manifest: Value = client
        .get(manifest_url)
        .send()?
        .error_for_status()?
        .json()?;
    let versions = manifest["versions"]
        .as_array()
        .ok_or_else(|| input_error("Minecraft API response has no versions list"))?;
    let version_entry = versions
        .iter()
        .find(|entry| entry["id"].as_str() == Some(version))
        .ok_or_else(|| input_error(format!("Minecraft version not found: {version}")))?;
    let version_url = version_entry["url"]
        .as_str()
        .ok_or_else(|| input_error("Version manifest entry has no URL"))?;
    let details: Value = client.get(version_url).send()?.error_for_status()?.json()?;
    let server_url = details["downloads"]["server"]["url"]
        .as_str()
        .ok_or_else(|| input_error(format!("Minecraft {version} has no server download")))?;
    let jar = client.get(server_url).send()?.error_for_status()?.bytes()?;
    fs::write(
        target.join(ServerKind::Vanilla.jar_name().expect("Vanilla uses a jar")),
        &jar,
    )?;
    Ok(())
}

fn install_fabric(
    config: &Config,
    target: &Path,
    minecraft_version: &str,
    api: &MinecraftApi,
    language: &Language,
) -> AppResult<()> {
    let client = http_client(config)?;
    let loader_versions_url = api
        .loader_versions_url
        .as_deref()
        .ok_or_else(|| input_error(format!("API {} has no Fabric loader endpoint", api.id)))?
        .replace("{minecraft_version}", minecraft_version);
    validate_https_url(&loader_versions_url)?;
    let loaders: Value = client
        .get(loader_versions_url)
        .send()?
        .error_for_status()?
        .json()?;
    let loader_version = loaders
        .as_array()
        .and_then(|items| items.first())
        .and_then(|item| item["loader"]["version"].as_str())
        .ok_or_else(|| input_error(format!("No Fabric loader found for {minecraft_version}")))?;

    let installer_versions_url = api
        .installer_versions_url
        .as_deref()
        .ok_or_else(|| input_error(format!("API {} has no Fabric installer endpoint", api.id)))?;
    validate_https_url(installer_versions_url)?;
    let installers: Value = client
        .get(installer_versions_url)
        .send()?
        .error_for_status()?
        .json()?;
    let installer_version = installers
        .as_array()
        .and_then(|items| items.first())
        .and_then(|item| item["version"].as_str())
        .ok_or_else(|| input_error("No Fabric installer version is available"))?;
    let installer_download_url = api
        .installer_download_url
        .as_deref()
        .ok_or_else(|| input_error(format!("API {} has no Fabric installer URL", api.id)))?
        .replace("{version}", installer_version);
    validate_https_url(&installer_download_url)?;
    let installer = client
        .get(installer_download_url)
        .send()?
        .error_for_status()?
        .bytes()?;
    let installer_path = target.join("fabric-installer.jar");
    fs::write(&installer_path, &installer)?;

    let java_executable = config.executable_path(&config.minecraft.java_executable);
    run_java_installer(
        &java_executable,
        target,
        &[
            "-jar",
            "fabric-installer.jar",
            "server",
            "-mcversion",
            minecraft_version,
            "-loader",
            loader_version,
            "-downloadMinecraft",
        ],
        "Fabric installer",
        language,
    )?;
    fs::remove_file(installer_path)?;
    Ok(())
}

fn install_forge(
    config: &Config,
    target: &Path,
    minecraft_version: &str,
    api: &MinecraftApi,
    language: &Language,
) -> AppResult<()> {
    let client = http_client(config)?;
    let promotions_url = api
        .forge_promotions_url
        .as_deref()
        .ok_or_else(|| input_error(format!("API {} has no Forge promotions endpoint", api.id)))?;
    let promotions: Value = client
        .get(promotions_url)
        .send()?
        .error_for_status()?
        .json()?;
    let forge_version = forge_version(&promotions, minecraft_version).ok_or_else(|| {
        input_error(format!(
            "No Forge version is available for Minecraft {minecraft_version}"
        ))
    })?;
    let installer_url = api
        .forge_installer_download_url
        .as_deref()
        .ok_or_else(|| input_error(format!("API {} has no Forge installer URL", api.id)))?
        .replace("{minecraft_version}", minecraft_version)
        .replace("{forge_version}", forge_version);
    validate_https_url(&installer_url)?;
    let installer = client
        .get(installer_url)
        .send()?
        .error_for_status()?
        .bytes()?;
    fs::write(target.join("forge-installer.jar"), &installer)?;
    run_java_installer(
        &config.executable_path(&config.minecraft.java_executable),
        target,
        &["-jar", "forge-installer.jar", "--installServer"],
        "Forge installer",
        language,
    )?;
    fs::remove_file(target.join("forge-installer.jar"))?;
    Ok(())
}

fn install_neoforge(
    config: &Config,
    target: &Path,
    minecraft_version: &str,
    api: &MinecraftApi,
    language: &Language,
) -> AppResult<()> {
    let client = http_client(config)?;
    let versions_url = api
        .neoforge_versions_url
        .as_deref()
        .ok_or_else(|| input_error(format!("API {} has no NeoForge versions endpoint", api.id)))?;
    let metadata = client
        .get(versions_url)
        .send()?
        .error_for_status()?
        .text()?;
    let installer_version = neoforge_version(&metadata, minecraft_version).ok_or_else(|| {
        input_error(format!(
            "No NeoForge version is available for Minecraft {minecraft_version}"
        ))
    })?;
    let installer_url = api
        .neoforge_installer_download_url
        .as_deref()
        .ok_or_else(|| input_error(format!("API {} has no NeoForge installer URL", api.id)))?
        .replace("{version}", installer_version);
    validate_https_url(&installer_url)?;
    let installer = client
        .get(installer_url)
        .send()?
        .error_for_status()?
        .bytes()?;
    fs::write(target.join("neoforge-installer.jar"), &installer)?;
    run_java_installer(
        &config.executable_path(&config.minecraft.java_executable),
        target,
        &["-jar", "neoforge-installer.jar", "--installServer"],
        "NeoForge installer",
        language,
    )?;
    fs::remove_file(target.join("neoforge-installer.jar"))?;
    Ok(())
}

fn install_paper(
    config: &Config,
    target: &Path,
    minecraft_version: &str,
    api: &MinecraftApi,
) -> AppResult<()> {
    let endpoint = api
        .paper_version_builds_url
        .as_deref()
        .ok_or_else(|| input_error(format!("API {} has no Paper builds endpoint", api.id)))?
        .replace("{minecraft_version}", minecraft_version);
    validate_https_url(&endpoint)?;
    let builds: Value = http_client(config)?
        .get(endpoint)
        .send()?
        .error_for_status()?
        .json()?;
    let download_url = paper_download_url(&builds).ok_or_else(|| {
        input_error(format!(
            "No Paper build found for Minecraft {minecraft_version}"
        ))
    })?;
    validate_https_url(download_url)?;
    let jar = http_client(config)?
        .get(download_url)
        .send()?
        .error_for_status()?
        .bytes()?;
    fs::write(
        target.join(ServerKind::Paper.jar_name().expect("Paper uses a jar")),
        &jar,
    )?;
    Ok(())
}

fn forge_version<'a>(promotions: &'a Value, minecraft_version: &str) -> Option<&'a str> {
    let promos = promotions["promos"].as_object()?;
    promos
        .get(&format!("{minecraft_version}-recommended"))
        .or_else(|| promos.get(&format!("{minecraft_version}-latest")))
        .and_then(Value::as_str)
}

fn paper_download_url(builds: &Value) -> Option<&str> {
    builds
        .as_array()
        .and_then(|items| items.first())
        .and_then(|build| build["downloads"]["server:default"]["url"].as_str())
}

fn neoforge_version<'a>(metadata: &'a str, minecraft_version: &str) -> Option<&'a str> {
    let prefix = neoforge_version_prefix(minecraft_version)?;
    metadata
        .split("<version>")
        .skip(1)
        .filter_map(|part| {
            part.split_once("</version>")
                .map(|(version, _)| version.trim())
        })
        .filter(|version| version.starts_with(&prefix))
        .max_by(|left, right| compare_versions(left, right))
}

fn neoforge_version_prefix(minecraft_version: &str) -> Option<String> {
    let parts = minecraft_version.split('.').collect::<Vec<_>>();
    if parts.len() < 2 {
        return None;
    }
    if parts[0] == "1" {
        if parts.len() >= 3 {
            Some(format!("{}.{}.", parts[1], parts[2]))
        } else {
            Some(format!("{}.0.", parts[1]))
        }
    } else {
        Some(format!("{minecraft_version}."))
    }
}

fn compare_versions(left: &str, right: &str) -> std::cmp::Ordering {
    let (left_release, left_suffix) = left.split_once(['-', '+']).unwrap_or((left, ""));
    let (right_release, right_suffix) = right.split_once(['-', '+']).unwrap_or((right, ""));
    let parts = |release: &str| {
        release
            .split('.')
            .map(|part| part.parse::<u64>().unwrap_or(0))
            .collect::<Vec<_>>()
    };
    parts(left_release)
        .cmp(&parts(right_release))
        .then_with(|| match (left_suffix.is_empty(), right_suffix.is_empty()) {
            (true, false) => std::cmp::Ordering::Greater,
            (false, true) => std::cmp::Ordering::Less,
            _ => left_suffix.cmp(right_suffix),
        })
}

fn run_java_installer(
    java_executable: &Path,
    target: &Path,
    args: &[&str],
    installer_name: &str,
    language: &Language,
) -> AppResult<()> {
    let status = Command::new(java_executable)
        .args(args)
        .current_dir(target)
        .status();
    let status = match status {
        Ok(status) => status,
        Err(error) => {
            return Err(input_error(language.format(
                "minecraft.java.launch_failed",
                &[
                    ("executable", &java_executable.display().to_string()),
                    ("error", &error.to_string()),
                ],
            )?));
        }
    };
    if !status.success() {
        return Err(input_error(format!(
            "{installer_name} failed with status {status}"
        )));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{
        ServerKind, compare_versions, forge_version, install, neoforge_version, paper_download_url,
        run_java_installer,
    };
    use crate::core::config::Config;
    use crate::core::language::Language;
    use crate::minecraft::api::MinecraftApi;
    use std::fs;
    use std::path::Path;
    use std::sync::atomic::{AtomicUsize, Ordering};

    fn temp_dir() -> std::path::PathBuf {
        static NEXT_ID: AtomicUsize = AtomicUsize::new(0);
        std::env::temp_dir().join(format!(
            "gsbuilder-fabric-installer-{}-{}",
            std::process::id(),
            NEXT_ID.fetch_add(1, Ordering::Relaxed)
        ))
    }

    #[test]
    fn parses_supported_server_kinds_and_default_apis() {
        assert_eq!(
            ServerKind::parse("vanilla").unwrap().default_api_id(),
            "mojang"
        );
        assert_eq!(
            ServerKind::parse("fabric").unwrap().default_api_id(),
            "fabric"
        );
        for kind in ["forge", "neoforge", "paper"] {
            assert_eq!(ServerKind::parse(kind).unwrap().provider_type(), kind);
        }
        assert!(ServerKind::parse("unknown").is_err());
    }

    #[test]
    fn selects_matching_forge_and_neoforge_versions() {
        let promotions = serde_json::json!({
            "promos": {
                "1.21.1-latest": "52.0.1",
                "1.21.1-recommended": "52.0.0"
            }
        });
        assert_eq!(forge_version(&promotions, "1.21.1"), Some("52.0.0"));
        let metadata = r#"
            <version>21.1.100</version>
            <version>21.1.9</version>
            <version>21.4.3</version>
        "#;
        assert_eq!(neoforge_version(metadata, "1.21.1"), Some("21.1.100"));
        assert_eq!(neoforge_version(metadata, "1.21.4"), Some("21.4.3"));
        assert_eq!(
            compare_versions("21.1.100", "21.1.9"),
            std::cmp::Ordering::Greater
        );
        assert_eq!(
            compare_versions("21.1.100", "21.1.100-beta"),
            std::cmp::Ordering::Greater
        );
    }

    #[test]
    fn selects_paper_build_download_url() {
        let builds = serde_json::json!([{
            "id": 133,
            "downloads": {
                "server:default": {
                    "name": "paper-1.21.1-133.jar",
                    "url": "https://example.com/paper.jar"
                }
            }
        }]);
        assert_eq!(
            paper_download_url(&builds),
            Some("https://example.com/paper.jar")
        );
        assert_eq!(paper_download_url(&serde_json::json!([])), None);
    }

    #[test]
    fn rejects_api_type_mismatch_before_network_or_file_access() {
        let config = Config::default();
        let api = MinecraftApi {
            id: "mojang".to_owned(),
            name: "Mojang".to_owned(),
            manifest_url: Some("https://example.com/versions.json".to_owned()),
            loader_versions_url: None,
            installer_versions_url: None,
            installer_download_url: None,
            forge_promotions_url: None,
            forge_installer_download_url: None,
            neoforge_versions_url: None,
            neoforge_installer_download_url: None,
            paper_project_url: None,
            paper_version_builds_url: None,
        };
        assert!(
            install(
                &config,
                Path::new("/path/that/does/not/exist"),
                "1.21.1",
                ServerKind::Fabric,
                &api,
                &Language::load(&config.language_file("ja").unwrap()).unwrap()
            )
            .is_err()
        );
    }

    #[test]
    fn explains_when_configured_java_executable_cannot_be_started() {
        let target = temp_dir();
        fs::create_dir_all(&target).unwrap();
        let language =
            Language::load(&Config::load().unwrap().language_file("ja").unwrap()).unwrap();
        let missing_java = target.join("java-that-does-not-exist");

        let error = run_java_installer(
            &missing_java,
            &target,
            &["-jar", "installer.jar"],
            "Test installer",
            &language,
        )
        .unwrap_err()
        .to_string();

        assert!(error.contains("Java"));
        assert!(error.contains("java_executable"));
        assert!(error.contains("java-that-does-not-exist"));
        fs::remove_dir_all(target).unwrap();
    }
}
