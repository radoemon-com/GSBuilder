use super::confirm_server_deletion;
use crate::core::config::Config;
use crate::core::error::{AppResult, input_error};
use crate::core::language::Language;
use crate::minecraft::MinecraftService;
use crate::steam::SteamService;
use crate::terraria::TerrariaService;
use std::path::Path;

pub(super) fn minecraft(config: &Config, language: &Language, args: &[String]) -> AppResult<()> {
    let service = MinecraftService::new(config, language);
    match args.first().map(String::as_str) {
        Some("server") => match args.get(1).map(String::as_str) {
            Some("create") if args.len() == 5 || args.len() == 6 => {
                create_minecraft_server(&service, &args[2..])
            }
            Some("delete") if args.len() == 3 => {
                if confirm_server_deletion(language, &args[2])? {
                    service.remove_server(&args[2])?;
                }
                Ok(())
            }
            _ => Err(input_error(
                "Usage: mc server create <type> <version> <name> [api_id] | delete <name>",
            )),
        },
        Some("api") => match args.get(1).map(String::as_str) {
            Some("list") if args.len() == 2 => service.list_apis(),
            Some("add") if args.len() >= 5 => {
                service.add_api(&args[2], &args[3], &args[4], &args[5..])
            }
            Some("remove") if args.len() == 3 => service.remove_api(&args[2]),
            _ => Err(input_error(
                "Usage: mc api list | add <provider_type> <api_id> <name> <endpoint_url...> | remove <api_id>",
            )),
        },
        Some("versions") if args.len() == 2 => service.list_versions(&args[1]),
        Some("loaders") if args.len() == 3 => service.list_fabric_loaders(&args[1], &args[2]),
        Some("create") if args.len() == 4 || args.len() == 5 => {
            create_minecraft_server(&service, &args[1..])
        }
        Some("mod") => match args.get(1).map(String::as_str) {
            Some("add") if args.len() == 4 => service.add_mods(&args[2], Path::new(&args[3])),
            _ => Err(input_error(
                "Usage: mc mod add <server> <source_file|directory|modpack.zip>",
            )),
        },
        _ => Err(input_error("Use 'help mc' for Minecraft commands")),
    }
}

fn create_minecraft_server(service: &MinecraftService<'_>, args: &[String]) -> AppResult<()> {
    service.create(
        &args[0],
        &args[1],
        &args[2],
        args.get(3).map(String::as_str),
    )
}

pub(super) fn steam(config: &Config, language: &Language, args: &[String]) -> AppResult<()> {
    let service = SteamService::new(config, language);
    match args.first().map(String::as_str) {
        Some("server") => match args.get(1).map(String::as_str) {
            Some("create") if args.len() == 4 || args.len() == 5 => {
                let (game, server, app_id) = steam_install_args(&args[2..]);
                service.install(game, server, app_id)
            }
            Some("delete") if args.len() == 4 => {
                if confirm_server_deletion(language, &args[3])? {
                    service.remove_server(&args[2], &args[3])?;
                }
                Ok(())
            }
            _ => Err(input_error(
                "Usage: steam server create <app_id> <name> | create <game> <name> <app_id> | delete <game> <name>",
            )),
        },
        Some("install") if args.len() == 3 || args.len() == 4 => {
            let (game, server, app_id) = steam_install_args(&args[1..]);
            service.install(game, server, app_id)
        }
        Some("mod") => match args.get(1).map(String::as_str) {
            Some("add") if args.len() == 5 || args.len() == 6 => service.add_mods(
                &args[2],
                &args[3],
                Path::new(&args[4]),
                args.get(5).map(String::as_str),
            ),
            Some("list") if args.len() == 4 || args.len() == 5 => {
                service.list_mods(&args[2], &args[3], args.get(4).map(String::as_str))
            }
            _ => Err(input_error("Usage: steam mod add|list <game> <server> ...")),
        },
        Some("workshop") => match args.get(1).map(String::as_str) {
            Some("item") if args.len() >= 5 => {
                service.install_workshop_item(&args[2], &args[3], &args[4..].join(" "))
            }
            Some("collection") if args.len() >= 5 => {
                service.install_workshop_collection(&args[2], &args[3], &args[4..].join(" "))
            }
            Some("remove") if args.len() >= 6 => match args[2].as_str() {
                "item" => service.remove_workshop_item(&args[3], &args[4], &args[5..].join(" ")),
                "collection" => {
                    service.remove_workshop_collection(&args[3], &args[4], &args[5..].join(" "))
                }
                _ => Err(input_error(
                    "Usage: steam workshop remove item|collection <game> <server_name> <id_or_url>[,...]",
                )),
            },
            _ => Err(input_error(
                "Usage: steam workshop item|collection <game> <server_name> <id_or_url>[,<id_or_url>...] | remove item|collection <game> <server_name> <id_or_url>[,<id_or_url>...]",
            )),
        },
        _ => Err(input_error("Use 'help steam' for Steam commands")),
    }
}

fn steam_install_args(args: &[String]) -> (&str, &str, &str) {
    if args.len() == 2 {
        (&args[0], &args[1], &args[0])
    } else {
        (&args[0], &args[1], &args[2])
    }
}

#[cfg(test)]
mod tests {
    use super::steam_install_args;

    #[test]
    fn maps_short_and_explicit_steam_server_creation_arguments() {
        assert_eq!(
            steam_install_args(&["222860".to_owned(), "test".to_owned()]),
            ("222860", "test", "222860")
        );
        assert_eq!(
            steam_install_args(&["Game".to_owned(), "test".to_owned(), "222860".to_owned()]),
            ("Game", "test", "222860")
        );
    }
}

pub(super) fn terraria(config: &Config, language: &Language, args: &[String]) -> AppResult<()> {
    let service = TerrariaService::new(config, language);
    match args.first().map(String::as_str) {
        Some("server") => match args.get(1).map(String::as_str) {
            Some("create") if args.len() == 4 => service.create(&args[2], &args[3]),
            Some("delete") if args.len() == 3 => {
                if confirm_server_deletion(language, &args[2])? {
                    service.remove_server(&args[2])?;
                }
                Ok(())
            }
            _ => Err(input_error(
                "Usage: terraria server create <type> <name> | delete <name>",
            )),
        },
        Some("create") if args.len() == 3 => service.create(&args[1], &args[2]),
        Some("mod") => match args.get(1).map(String::as_str) {
            Some("add") if args.len() == 5 => {
                service.add_workshop_mod(&args[2], &args[3], &args[4])
            }
            Some("add-file") if args.len() == 4 => {
                service.add_mod_file(&args[2], Path::new(&args[3]))
            }
            _ => Err(input_error(
                "Usage: terraria mod add <server> <workshop_id> <mod_name> | add-file <server> <file.tmod>",
            )),
        },
        _ => Err(input_error("Use 'help terraria' for Terraria commands")),
    }
}
