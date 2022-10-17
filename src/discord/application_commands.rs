use twilight_model::application::command::{BaseCommandOptionData, ChoiceCommandOptionData, Command, CommandOption, CommandType, NumberCommandOptionData};
use twilight_model::guild::Permissions;
use twilight_util::builder::command::CommandBuilder;
use crate::constants::WEBSITE_URL;
use crate::discord::{CANCEL_RACE_CMD, CREATE_BRACKET_CMD, CREATE_RACE_CMD, CREATE_SEASON_CMD};

pub fn application_command_definitions() -> Vec<Command> {
    let create_race = CommandBuilder::new(
        CREATE_RACE_CMD.to_string(),
        "Create an asynchronous race for two players".to_string(),
        CommandType::ChatInput,
    )
        .default_member_permissions(Permissions::MANAGE_GUILD)
        .option(CommandOption::User(BaseCommandOptionData {
            description: "First racer".to_string(),
            description_localizations: None,
            name: "p1".to_string(),
            name_localizations: None,
            required: true,
        }))
        .option(CommandOption::User(BaseCommandOptionData {
            description: "Second racer".to_string(),
            description_localizations: None,
            name: "p2".to_string(),
            name_localizations: None,
            required: true,
        }))
        .build();

    let cancel_race = CommandBuilder::new(
        CANCEL_RACE_CMD.to_string(),
        "Cancel an existing asynchronous race".to_string(),
        CommandType::ChatInput,
    )
        .default_member_permissions(Permissions::MANAGE_GUILD)
        .option(CommandOption::Integer(NumberCommandOptionData {
            autocomplete: false,
            choices: vec![],
            description: format!("Race ID. Get this from {}", WEBSITE_URL),
            description_localizations: None,
            max_value: None,
            min_value: None,
            name: "race_id".to_string(),
            name_localizations: None,
            required: true,
        }))
        .build();

    let create_season = CommandBuilder::new(
        CREATE_SEASON_CMD.to_string(),
        "Create a new season".to_string(),
        CommandType::ChatInput,
    )
        .default_member_permissions(Permissions::MANAGE_GUILD)
        .option(CommandOption::String(ChoiceCommandOptionData {
            autocomplete: false,
            choices: vec![],
            description: "Format (e.g. Any% NMG)".to_string(),
            description_localizations: None,
            max_length: None,
            min_length: None,
            name: "format".to_string(),
            name_localizations: None,
            required: true,
        }))
        .build();

    let create_bracket = CommandBuilder::new(
        CREATE_BRACKET_CMD.to_string(),
        "Create a new bracket".to_string(),
        CommandType::ChatInput,
    )
        .default_member_permissions(Permissions::MANAGE_GUILD)
        .option(CommandOption::String(ChoiceCommandOptionData {
            autocomplete: false,
            choices: vec![],
            description: "Name (e.g. Dark World)".to_string(),
            description_localizations: None,
            max_length: None,
            min_length: None,
            name: "name".to_string(),
            name_localizations: None,
            required: true,
        }))
        .option(CommandOption::Integer(NumberCommandOptionData {
            autocomplete: false,
            choices: vec![],
            description: "Season ID".to_string(),
            description_localizations: None,
            max_value: None,
            min_value: None,
            name: "season_id".to_string(),
            name_localizations: None,
            required: true,
        }))
        .build();

    vec![create_race, cancel_race, create_season, create_bracket]

}