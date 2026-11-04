use crate::core::config::Config;
use crate::core::error::{AppResult, input_error};
use crate::core::http::http_client;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::fs;
use std::path::Path;

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MinecraftApiRegistry {
    pub apis: Vec<MinecraftApi>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MinecraftApi {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub manifest_url: Option<String>,
    #[serde(default)]
    pub loader_versions_url: Option<String>,
    #[serde(default)]
    pub installer_versions_url: Option<String>,
    #[serde(default)]
    pub installer_download_url: Option<String>,
    #[serde(default)]
    pub forge_promotions_url: Option<String>,
    #[serde(default)]
    pub forge_installer_download_url: Option<String>,
    #[serde(default)]
    pub neoforge_versions_url: Option<String>,
    #[serde(default)]
    pub neoforge_installer_download_url: Option<String>,
    #[serde(default)]
    pub paper_project_url: Option<String>,
    #[serde(default)]
    pub paper_version_builds_url: Option<String>,
}

impl MinecraftApi {
    pub fn provider_type(&self) -> &'static str {
        if self.manifest_url.is_some() {
            "vanilla"
        } else if self.loader_versions_url.is_some() {
            "fabric"
        } else if self.forge_promotions_url.is_some() {
            "forge"
        } else if self.neoforge_versions_url.is_some() {
            "neoforge"
        } else {
            "paper"
        }
    }
}

impl MinecraftApiRegistry {
    pub fn load(config: &Config) -> AppResult<Self> {
        let path = config.minecraft_api_registry_path();
        if !path.exists() {
            let registry = Self {
                apis: vec![
                    mojang_api(),
                    fabric_api(),
                    forge_api(),
                    neoforge_api(),
                    paper_api(),
                ],
            };
            registry.save_to(&path)?;
            return Ok(registry);
        }
        let contents = fs::read_to_string(&path)?;
        let registry: Self = serde_json::from_str(&contents).map_err(|error| {
            input_error(format!(
                "Invalid Minecraft API registry {}: {error}",
                path.display()
            ))
        })?;
        registry.validate()?;
        Ok(registry)
    }

    pub fn get(&self, id: &str) -> AppResult<&MinecraftApi> {
        self.apis
            .iter()
            .find(|api| api.id == id)
            .ok_or_else(|| input_error(format!("Minecraft API not found: {id}")))
    }

    pub fn add(&mut self, config: &Config, api: MinecraftApi) -> AppResult<()> {
        validate_api(&api)?;
        if self.apis.iter().any(|existing| existing.id == api.id) {
            return Err(input_error(format!(
                "Minecraft API ID already exists: {}",
                api.id
            )));
        }
        self.apis.push(api);
        self.save(config)
    }

    pub fn add_provider(
        &mut self,
        config: &Config,
        provider_type: &str,
        id: &str,
        name: &str,
        endpoint_urls: &[String],
    ) -> AppResult<()> {
        let endpoint_count = match provider_type {
            "vanilla" => 1,
            "fabric" => 3,
            "forge" | "neoforge" | "paper" => 2,
            _ => {
                return Err(input_error(format!(
                    "Unsupported Minecraft API provider type: {provider_type} (supported: vanilla, fabric, forge, neoforge, paper)"
                )));
            }
        };
        if endpoint_urls.len() != endpoint_count {
            return Err(input_error(format!(
                "Provider type {provider_type} requires {endpoint_count} endpoint URL(s)"
            )));
        }
        let api = MinecraftApi {
            id: id.to_owned(),
            name: name.to_owned(),
            manifest_url: (provider_type == "vanilla").then(|| endpoint_urls[0].clone()),
            loader_versions_url: (provider_type == "fabric").then(|| endpoint_urls[0].clone()),
            installer_versions_url: (provider_type == "fabric").then(|| endpoint_urls[1].clone()),
            installer_download_url: (provider_type == "fabric").then(|| endpoint_urls[2].clone()),
            forge_promotions_url: (provider_type == "forge").then(|| endpoint_urls[0].clone()),
            forge_installer_download_url: (provider_type == "forge")
                .then(|| endpoint_urls[1].clone()),
            neoforge_versions_url: (provider_type == "neoforge").then(|| endpoint_urls[0].clone()),
            neoforge_installer_download_url: (provider_type == "neoforge")
                .then(|| endpoint_urls[1].clone()),
            paper_project_url: (provider_type == "paper").then(|| endpoint_urls[0].clone()),
            paper_version_builds_url: (provider_type == "paper").then(|| endpoint_urls[1].clone()),
        };
        self.add(config, api)
    }

    pub fn remove(&mut self, config: &Config, id: &str) -> AppResult<()> {
        let Some(index) = self.apis.iter().position(|api| api.id == id) else {
            return Err(input_error(format!("Minecraft API not found: {id}")));
        };
        self.apis.remove(index);
        self.save(config)
    }

    fn validate(&self) -> AppResult<()> {
        for (index, api) in self.apis.iter().enumerate() {
            validate_api(api)?;
            if self.apis[index + 1..]
                .iter()
                .any(|other| other.id == api.id)
            {
                return Err(input_error(format!(
                    "Duplicate Minecraft API ID: {}",
                    api.id
                )));
            }
        }
        Ok(())
    }

    fn save(&self, config: &Config) -> AppResult<()> {
        self.save_to(&config.minecraft_api_registry_path())
    }

    fn save_to(&self, path: &Path) -> AppResult<()> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        let temporary_path = path.with_extension("json.tmp");
        let contents = serde_json::to_string_pretty(self)?;
        fs::write(&temporary_path, format!("{contents}\n"))?;
        fs::rename(temporary_path, path)?;
        Ok(())
    }
}

pub fn fetch_versions(config: &Config, api: &MinecraftApi) -> AppResult<Vec<String>> {
    let manifest_url = api
        .manifest_url
        .as_deref()
        .ok_or_else(|| input_error(format!("API {} is not a vanilla manifest API", api.id)))?;
    let manifest: Value = http_client(config)?
        .get(manifest_url)
        .send()?
        .error_for_status()?
        .json()?;
    let versions = manifest["versions"]
        .as_array()
        .ok_or_else(|| input_error("Minecraft API response has no versions list"))?
        .iter()
        .filter_map(|entry| entry["id"].as_str().map(str::to_owned))
        .collect();
    Ok(versions)
}

pub fn fetch_fabric_loaders(
    config: &Config,
    api: &MinecraftApi,
    minecraft_version: &str,
) -> AppResult<Vec<String>> {
    let endpoint = api
        .loader_versions_url
        .as_deref()
        .ok_or_else(|| input_error(format!("API {} has no Fabric loader endpoint", api.id)))?;
    let url = endpoint.replace("{minecraft_version}", minecraft_version);
    validate_https_url(&url)?;
    let loaders: Value = http_client(config)?
        .get(url)
        .send()?
        .error_for_status()?
        .json()?;
    let versions = loaders
        .as_array()
        .ok_or_else(|| input_error("Fabric API response has no loader list"))?
        .iter()
        .filter_map(|entry| entry["loader"]["version"].as_str().map(str::to_owned))
        .collect();
    Ok(versions)
}

fn mojang_api() -> MinecraftApi {
    MinecraftApi {
        id: "mojang".to_owned(),
        name: "Mojang".to_owned(),
        manifest_url: Some(
            "https://piston-meta.mojang.com/mc/game/version_manifest_v2.json".to_owned(),
        ),
        loader_versions_url: None,
        installer_versions_url: None,
        installer_download_url: None,
        forge_promotions_url: None,
        forge_installer_download_url: None,
        neoforge_versions_url: None,
        neoforge_installer_download_url: None,
        paper_project_url: None,
        paper_version_builds_url: None,
    }
}

fn fabric_api() -> MinecraftApi {
    MinecraftApi {
        id: "fabric".to_owned(),
        name: "Fabric".to_owned(),
        manifest_url: None,
        loader_versions_url: Some(
            "https://meta.fabricmc.net/v2/versions/loader/{minecraft_version}".to_owned(),
        ),
        installer_versions_url: Some("https://meta.fabricmc.net/v2/versions/installer".to_owned()),
        installer_download_url: Some(
            "https://maven.fabricmc.net/net/fabricmc/fabric-installer/{version}/fabric-installer-{version}.jar"
                .to_owned(),
        ),
        forge_promotions_url: None,
        forge_installer_download_url: None,
        neoforge_versions_url: None,
        neoforge_installer_download_url: None,
        paper_project_url: None,
        paper_version_builds_url: None,
    }
}

fn forge_api() -> MinecraftApi {
    MinecraftApi {
        id: "forge".to_owned(),
        name: "Minecraft Forge".to_owned(),
        manifest_url: None,
        loader_versions_url: None,
        installer_versions_url: None,
        installer_download_url: None,
        forge_promotions_url: Some(
            "https://files.minecraftforge.net/net/minecraftforge/forge/promotions_slim.json"
                .to_owned(),
        ),
        forge_installer_download_url: Some(
            "https://maven.minecraftforge.net/net/minecraftforge/forge/{minecraft_version}-{forge_version}/forge-{minecraft_version}-{forge_version}-installer.jar"
                .to_owned(),
        ),
        neoforge_versions_url: None,
        neoforge_installer_download_url: None,
        paper_project_url: None,
        paper_version_builds_url: None,
    }
}

fn neoforge_api() -> MinecraftApi {
    MinecraftApi {
        id: "neoforge".to_owned(),
        name: "NeoForge".to_owned(),
        manifest_url: None,
        loader_versions_url: None,
        installer_versions_url: None,
        installer_download_url: None,
        forge_promotions_url: None,
        forge_installer_download_url: None,
        neoforge_versions_url: Some(
            "https://maven.neoforged.net/releases/net/neoforged/neoforge/maven-metadata.xml"
                .to_owned(),
        ),
        neoforge_installer_download_url: Some(
            "https://maven.neoforged.net/releases/net/neoforged/neoforge/{version}/neoforge-{version}-installer.jar"
                .to_owned(),
        ),
        paper_project_url: None,
        paper_version_builds_url: None,
    }
}

fn paper_api() -> MinecraftApi {
    MinecraftApi {
        id: "paper".to_owned(),
        name: "PaperMC".to_owned(),
        manifest_url: None,
        loader_versions_url: None,
        installer_versions_url: None,
        installer_download_url: None,
        forge_promotions_url: None,
        forge_installer_download_url: None,
        neoforge_versions_url: None,
        neoforge_installer_download_url: None,
        paper_project_url: Some("https://fill.papermc.io/v3/projects/paper".to_owned()),
        paper_version_builds_url: Some(
            "https://fill.papermc.io/v3/projects/paper/versions/{minecraft_version}/builds"
                .to_owned(),
        ),
    }
}

fn validate_api(api: &MinecraftApi) -> AppResult<()> {
    if api.id.is_empty()
        || !api
            .id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
    {
        return Err(input_error(
            "Minecraft API IDs must contain only letters, digits, '-' or '_'",
        ));
    }
    if api.name.trim().is_empty() {
        return Err(input_error("Minecraft API name cannot be empty"));
    }
    let vanilla = api.manifest_url.is_some()
        && api.loader_versions_url.is_none()
        && api.installer_versions_url.is_none()
        && api.installer_download_url.is_none()
        && api.forge_promotions_url.is_none()
        && api.forge_installer_download_url.is_none()
        && api.neoforge_versions_url.is_none()
        && api.neoforge_installer_download_url.is_none()
        && api.paper_project_url.is_none()
        && api.paper_version_builds_url.is_none();
    let fabric = api.manifest_url.is_none()
        && api.loader_versions_url.is_some()
        && api.installer_versions_url.is_some()
        && api.installer_download_url.is_some()
        && api.forge_promotions_url.is_none()
        && api.forge_installer_download_url.is_none()
        && api.neoforge_versions_url.is_none()
        && api.neoforge_installer_download_url.is_none()
        && api.paper_project_url.is_none()
        && api.paper_version_builds_url.is_none();
    let forge = api.manifest_url.is_none()
        && api.loader_versions_url.is_none()
        && api.installer_versions_url.is_none()
        && api.installer_download_url.is_none()
        && api.forge_promotions_url.is_some()
        && api.forge_installer_download_url.is_some()
        && api.neoforge_versions_url.is_none()
        && api.neoforge_installer_download_url.is_none()
        && api.paper_project_url.is_none()
        && api.paper_version_builds_url.is_none();
    let neoforge = api.manifest_url.is_none()
        && api.loader_versions_url.is_none()
        && api.installer_versions_url.is_none()
        && api.installer_download_url.is_none()
        && api.forge_promotions_url.is_none()
        && api.forge_installer_download_url.is_none()
        && api.neoforge_versions_url.is_some()
        && api.neoforge_installer_download_url.is_some()
        && api.paper_project_url.is_none()
        && api.paper_version_builds_url.is_none();
    let paper = api.manifest_url.is_none()
        && api.loader_versions_url.is_none()
        && api.installer_versions_url.is_none()
        && api.installer_download_url.is_none()
        && api.forge_promotions_url.is_none()
        && api.forge_installer_download_url.is_none()
        && api.neoforge_versions_url.is_none()
        && api.neoforge_installer_download_url.is_none()
        && api.paper_project_url.is_some()
        && api.paper_version_builds_url.is_some();
    if !vanilla && !fabric && !forge && !neoforge && !paper {
        return Err(input_error(
            "Minecraft API must define endpoints for exactly one supported provider",
        ));
    }
    if fabric
        && (!api
            .loader_versions_url
            .as_deref()
            .is_some_and(|url| url.contains("{minecraft_version}"))
            || !api
                .installer_download_url
                .as_deref()
                .is_some_and(|url| url.contains("{version}")))
    {
        return Err(input_error(
            "Fabric API URLs must include {minecraft_version} and {version} placeholders",
        ));
    }
    if forge
        && !api
            .forge_installer_download_url
            .as_deref()
            .is_some_and(|url| {
                url.contains("{minecraft_version}") && url.contains("{forge_version}")
            })
    {
        return Err(input_error(
            "Forge installer URL must include {minecraft_version} and {forge_version} placeholders",
        ));
    }
    if neoforge
        && !api
            .neoforge_installer_download_url
            .as_deref()
            .is_some_and(|url| url.contains("{version}"))
    {
        return Err(input_error(
            "NeoForge installer URL must include the {version} placeholder",
        ));
    }
    if paper
        && !api
            .paper_version_builds_url
            .as_deref()
            .is_some_and(|url| url.contains("{minecraft_version}"))
    {
        return Err(input_error(
            "Paper build URL must include the {minecraft_version} placeholder",
        ));
    }
    for endpoint in [
        api.manifest_url.as_deref(),
        api.loader_versions_url.as_deref(),
        api.installer_versions_url.as_deref(),
        api.installer_download_url.as_deref(),
        api.forge_promotions_url.as_deref(),
        api.forge_installer_download_url.as_deref(),
        api.neoforge_versions_url.as_deref(),
        api.neoforge_installer_download_url.as_deref(),
        api.paper_project_url.as_deref(),
        api.paper_version_builds_url.as_deref(),
    ]
    .into_iter()
    .flatten()
    {
        validate_https_url(endpoint)?;
    }
    Ok(())
}

pub(super) fn validate_https_url(value: &str) -> AppResult<()> {
    let url = reqwest::Url::parse(value)
        .map_err(|error| input_error(format!("Invalid Minecraft API URL: {error}")))?;
    if url.scheme() != "https" {
        return Err(input_error("Minecraft API URLs must use HTTPS"));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{MinecraftApi, MinecraftApiRegistry, validate_api};
    use crate::core::config::Config;
    use std::fs;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicUsize, Ordering};

    fn test_config() -> (Config, PathBuf) {
        static NEXT_ID: AtomicUsize = AtomicUsize::new(0);
        let directory = std::env::temp_dir().join(format!(
            "gsbuilder-api-test-{}-{}",
            std::process::id(),
            NEXT_ID.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir_all(&directory).unwrap();
        let config = Config::from_contents(
            r#"
[minecraft]
api_registry = "apis.json"
"#,
            directory.clone(),
        )
        .unwrap();
        (config, directory)
    }

    #[test]
    fn creates_default_vanilla_fabric_forge_neoforge_and_paper_apis() {
        let (config, directory) = test_config();
        let mut registry = MinecraftApiRegistry::load(&config).unwrap();
        assert!(registry.get("mojang").unwrap().manifest_url.is_some());
        assert!(
            registry
                .get("fabric")
                .unwrap()
                .loader_versions_url
                .is_some()
        );
        assert_eq!(registry.get("forge").unwrap().provider_type(), "forge");
        assert_eq!(
            registry.get("neoforge").unwrap().provider_type(),
            "neoforge"
        );
        assert_eq!(registry.get("paper").unwrap().provider_type(), "paper");
        assert_eq!(
            registry.get("paper").unwrap().paper_project_url.as_deref(),
            Some("https://fill.papermc.io/v3/projects/paper")
        );
        assert!(
            registry
                .get("forge")
                .unwrap()
                .forge_promotions_url
                .is_some()
        );
        assert!(
            registry
                .get("neoforge")
                .unwrap()
                .neoforge_versions_url
                .is_some()
        );
        registry
            .add_provider(
                &config,
                "vanilla",
                "mirror",
                "Mirror",
                &["https://example.com/versions.json".to_owned()],
            )
            .unwrap();
        assert!(
            MinecraftApiRegistry::load(&config)
                .unwrap()
                .get("mirror")
                .is_ok()
        );
        registry.remove(&config, "mirror").unwrap();
        assert!(
            MinecraftApiRegistry::load(&config)
                .unwrap()
                .get("mirror")
                .is_err()
        );
        for (provider_type, id, endpoint_urls) in [
            (
                "fabric",
                "fabric-mirror",
                vec![
                    "https://example.com/loader/{minecraft_version}".to_owned(),
                    "https://example.com/installer/versions".to_owned(),
                    "https://example.com/installer/{version}.jar".to_owned(),
                ],
            ),
            (
                "forge",
                "forge-mirror",
                vec![
                    "https://example.com/forge/promotions".to_owned(),
                    "https://example.com/forge/{minecraft_version}-{forge_version}.jar".to_owned(),
                ],
            ),
            (
                "neoforge",
                "neoforge-mirror",
                vec![
                    "https://example.com/neoforge/versions".to_owned(),
                    "https://example.com/neoforge/{version}.jar".to_owned(),
                ],
            ),
            (
                "paper",
                "paper-mirror",
                vec![
                    "https://example.com/paper".to_owned(),
                    "https://example.com/paper/{minecraft_version}/builds".to_owned(),
                ],
            ),
        ] {
            registry
                .add_provider(&config, provider_type, id, id, &endpoint_urls)
                .unwrap();
            assert_eq!(registry.get(id).unwrap().provider_type(), provider_type);
        }
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn only_accepts_complete_https_api_definitions() {
        let vanilla = MinecraftApi {
            id: "test".to_owned(),
            name: "Test".to_owned(),
            manifest_url: Some("http://example.com/versions.json".to_owned()),
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
        assert!(validate_api(&vanilla).is_err());
        let incomplete_fabric = MinecraftApi {
            id: "fabric-custom".to_owned(),
            name: "Fabric mirror".to_owned(),
            manifest_url: None,
            loader_versions_url: Some("https://example.com/{minecraft_version}".to_owned()),
            installer_versions_url: None,
            installer_download_url: None,
            forge_promotions_url: None,
            forge_installer_download_url: None,
            neoforge_versions_url: None,
            neoforge_installer_download_url: None,
            paper_project_url: None,
            paper_version_builds_url: None,
        };
        assert!(validate_api(&incomplete_fabric).is_err());
    }
}
