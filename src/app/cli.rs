use crate::core::config::Config;
use crate::core::error::{AppResult, input_error};
use crate::core::language::Language;
use crate::minecraft::MinecraftService;
use crate::steam::SteamService;
use rustyline::DefaultEditor;
use rustyline::error::ReadlineError;
use std::path::Path;

pub fn run(config: &Config, language: &Language) -> AppResult<()> {
    let mut editor = DefaultEditor::new()?;
    loop {
        let line = match editor.readline(language.text("cli.prompt")?) {
            Ok(line) => line,
            Err(ReadlineError::Interrupted) => {
                println!();
                continue;
            }
            Err(ReadlineError::Eof) => break,
            Err(error) => return Err(Box::new(error)),
        };
        let words = match tokenize(&line) {
            Ok(words) => words,
            Err(_) => {
                eprintln!("{}", language.text("cli.parse.unclosed_quote")?);
                editor.add_history_entry(&line)?;
                continue;
            }
        };
        if words.is_empty() {
            continue;
        }
        editor.add_history_entry(&line)?;
        match dispatch(config, language, &words) {
            Ok(true) => {}
            Ok(false) => break,
            Err(error) => eprintln!(
                "{}",
                language.format("cli.error", &[("error", &error.to_string())])?
            ),
        }
    }
    println!("{}", language.text("cli.goodbye")?);
    Ok(())
}

fn dispatch(config: &Config, language: &Language, words: &[String]) -> AppResult<bool> {
    match words[0].as_str() {
        "help" | "?" => {
            let (topic_args, verbose) = parse_help_args(&words[1..]);
            let Some(key) = help_key(&topic_args, verbose) else {
                let topic = topic_args.join(" ");
                return Err(input_error(
                    language.format("cli.help.topic.unknown", &[("topic", &topic)])?,
                ));
            };
            println!("{}", language.text(key)?);
            Ok(true)
        }
        "exit" | "quit" => Ok(false),
        "mc" | "minecraft" => {
            dispatch_minecraft(config, language, &words[1..])?;
            Ok(true)
        }
        "steam" => {
            dispatch_steam(config, language, &words[1..])?;
            Ok(true)
        }
        command => Err(input_error(
            language.format("cli.command.unknown", &[("command", command)])?,
        )),
    }
}

fn parse_help_args(args: &[String]) -> (Vec<String>, bool) {
    let verbose = args
        .iter()
        .any(|arg| matches!(arg.as_str(), "--verbose" | "-v"));
    let topic = args
        .iter()
        .filter(|arg| !matches!(arg.as_str(), "--verbose" | "-v"))
        .cloned()
        .collect();
    (topic, verbose)
}

fn help_key(args: &[String], verbose: bool) -> Option<&'static str> {
    let topic = args.iter().map(String::as_str).collect::<Vec<_>>();
    let (short, detailed) = match topic.as_slice() {
        [] | ["all"] => ("cli.help.short", "cli.help"),
        ["mc" | "minecraft"] => ("cli.help.short.mc", "cli.help.minecraft"),
        ["mc" | "minecraft", "api"] | ["mc" | "minecraft", "api", "list" | "remove"] => {
            ("cli.help.short.mc.api", "cli.help.mc.api")
        }
        ["mc" | "minecraft", "api", "add"] | ["mc" | "minecraft", "api", "add", _] => {
            ("cli.help.short.mc.api.add", "cli.help.mc.api.add")
        }
        ["mc" | "minecraft", "versions"] => ("cli.help.short.mc.versions", "cli.help.mc.versions"),
        ["mc" | "minecraft", "loaders"] => ("cli.help.short.mc.loaders", "cli.help.mc.loaders"),
        ["mc" | "minecraft", "create"] => ("cli.help.short.mc.create", "cli.help.mc.create"),
        ["mc" | "minecraft", "mod"] | ["mc" | "minecraft", "mod", "add"] => {
            ("cli.help.short.mc.mod", "cli.help.mc.mod")
        }
        ["steam"] => ("cli.help.short.steam", "cli.help.steam"),
        ["steam", "install"] => ("cli.help.short.steam.install", "cli.help.steam.install"),
        ["steam", "mod"] => ("cli.help.short.steam.mod", "cli.help.steam.mod"),
        ["steam", "mod", "add"] => ("cli.help.short.steam.mod.add", "cli.help.steam.mod.add"),
        ["steam", "mod", "list"] => ("cli.help.short.steam.mod.list", "cli.help.steam.mod.list"),
        ["steam", "workshop"]
        | ["steam", "workshop", "item" | "collection" | "remove"]
        | ["steam", "workshop", "remove", "item" | "collection"] => {
            ("cli.help.short.steam.workshop", "cli.help.steam.workshop")
        }
        _ => return None,
    };
    Some(if verbose { detailed } else { short })
}

fn dispatch_minecraft(config: &Config, language: &Language, args: &[String]) -> AppResult<()> {
    let service = MinecraftService::new(config, language);
    match args.first().map(String::as_str) {
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
        Some("create") if (args.len() == 4 || args.len() == 5) => service.create(
            &args[1],
            &args[2],
            &args[3],
            args.get(4).map(String::as_str),
        ),
        Some("mod") => match args.get(1).map(String::as_str) {
            Some("add") if args.len() == 4 => service.add_mods(&args[2], Path::new(&args[3])),
            _ => Err(input_error("Usage: mc mod add <server> <source_directory>")),
        },
        _ => Err(input_error("Use 'help mc' for Minecraft commands")),
    }
}

fn dispatch_steam(config: &Config, language: &Language, args: &[String]) -> AppResult<()> {
    let service = SteamService::new(config, language);
    match args.first().map(String::as_str) {
        Some("install") if args.len() == 3 || args.len() == 4 => {
            let (game, server, app_id) = steam_install_args(args);
            service.install(game, server, app_id)
        }
        Some("mod") => match args.get(1).map(String::as_str) {
            Some("add") if (args.len() == 5 || args.len() == 6) => service.add_mods(
                &args[2],
                &args[3],
                Path::new(&args[4]),
                args.get(5).map(String::as_str),
            ),
            Some("list") if (args.len() == 4 || args.len() == 5) => {
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
    if args.len() == 3 {
        (&args[1], &args[2], &args[1])
    } else {
        (&args[1], &args[2], &args[3])
    }
}

fn tokenize(line: &str) -> AppResult<Vec<String>> {
    let mut tokens = Vec::new();
    let mut token = String::new();
    let mut quote = None;
    let mut chars = line.chars().peekable();
    let mut token_started = false;

    while let Some(character) = chars.next() {
        match (quote, character) {
            (Some(current), character) if character == current => quote = None,
            (Some('"'), '\\') if matches!(chars.peek(), Some('"') | Some('\\')) => {
                token.push(chars.next().expect("peeked character exists"));
            }
            (Some(_), character) => token.push(character),
            (None, '"' | '\'') => {
                quote = Some(character);
                token_started = true;
            }
            (None, character) if character.is_whitespace() => {
                if token_started {
                    tokens.push(std::mem::take(&mut token));
                    token_started = false;
                }
            }
            (None, character) => {
                token.push(character);
                token_started = true;
            }
        }
    }
    if quote.is_some() {
        return Err(input_error("unclosed quote"));
    }
    if token_started {
        tokens.push(token);
    }
    Ok(tokens)
}

#[cfg(test)]
mod tests {
    use super::{help_key, parse_help_args, steam_install_args, tokenize};

    #[test]
    fn parses_quoted_paths_and_empty_quoted_arguments() {
        assert_eq!(
            tokenize(r#"steam mod add "Core Keeper" "My Server" "mods folder" "#).unwrap(),
            [
                "steam",
                "mod",
                "add",
                "Core Keeper",
                "My Server",
                "mods folder"
            ]
        );
        assert_eq!(
            tokenize(r#"mc api add x "" https://example.com"#).unwrap()[4],
            ""
        );
    }

    #[test]
    fn rejects_unclosed_quotes() {
        assert!(tokenize(r#"mc create "vanilla"#).is_err());
    }

    #[test]
    fn resolves_short_and_verbose_help_topics() {
        assert_eq!(
            help_key(&["steam".to_owned(), "install".to_owned()], false),
            Some("cli.help.short.steam.install")
        );
        assert_eq!(
            help_key(&["mc".to_owned(), "create".to_owned()], true),
            Some("cli.help.mc.create")
        );
        assert_eq!(
            help_key(
                &[
                    "mc".to_owned(),
                    "api".to_owned(),
                    "add".to_owned(),
                    "fabric".to_owned()
                ],
                true
            ),
            Some("cli.help.mc.api.add")
        );
        assert_eq!(
            help_key(
                &["steam".to_owned(), "workshop".to_owned(), "item".to_owned()],
                true
            ),
            Some("cli.help.steam.workshop")
        );
        assert_eq!(
            help_key(
                &[
                    "steam".to_owned(),
                    "workshop".to_owned(),
                    "remove".to_owned(),
                    "item".to_owned()
                ],
                true
            ),
            Some("cli.help.steam.workshop")
        );
        assert_eq!(help_key(&["unknown".to_owned()], false), None);
    }

    #[test]
    fn parses_verbose_flag_before_or_after_help_topic() {
        assert_eq!(
            parse_help_args(&[
                "steam".to_owned(),
                "install".to_owned(),
                "--verbose".to_owned()
            ]),
            (vec!["steam".to_owned(), "install".to_owned()], true)
        );
        assert_eq!(
            parse_help_args(&["-v".to_owned(), "mc".to_owned()]),
            (vec!["mc".to_owned()], true)
        );
    }

    #[test]
    fn supports_short_and_explicit_steam_install_arguments() {
        let short = vec!["install".to_owned(), "222860".to_owned(), "test".to_owned()];
        assert_eq!(steam_install_args(&short), ("222860", "test", "222860"));

        let explicit = vec![
            "install".to_owned(),
            "Game".to_owned(),
            "test".to_owned(),
            "222860".to_owned(),
        ];
        assert_eq!(steam_install_args(&explicit), ("Game", "test", "222860"));
    }
}
