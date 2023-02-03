use crate::discord::constants::{
    ADD_PLAYER_TO_BRACKET_CMD, CANCEL_RACE_CMD, CHECK_USER_INFO_CMD, CREATE_BRACKET_CMD,
    CREATE_PLAYER_CMD, CREATE_RACE_CMD, CREATE_SEASON_CMD, FINISH_BRACKET_CMD,
    GENERATE_PAIRINGS_CMD, REPORT_RACE_CMD, RESCHEDULE_RACE_CMD, SCHEDULE_RACE_CMD,
    SET_SEASON_STATE_CMD, SUBMIT_QUALIFIER_CMD, UPDATE_FINISHED_RACE_CMD, UPDATE_USER_INFO_CMD,
};
use enum_iterator::all;
use nmg_league_bot::constants::WEBSITE_URL;
use nmg_league_bot::models::season::SeasonState;
use twilight_model::application::command::{
    BaseCommandOptionData, ChoiceCommandOptionData, Command, CommandOption, CommandOptionChoice,
    CommandOptionValue, CommandType, NumberCommandOptionData,
};
use twilight_model::guild::Permissions;
use twilight_util::builder::command::CommandBuilder;

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

    let possible_states = all::<SeasonState>()
        .flat_map(|s| serde_json::to_string(&s))
        .map(|s| CommandOptionChoice::String {
            name: s.clone(),
            name_localizations: None,
            value: s,
        })
        .collect();

    let set_season_state = CommandBuilder::new(
        SET_SEASON_STATE_CMD.to_string(),
        "Set a season's state".to_string(),
        CommandType::ChatInput,
    )
    .default_member_permissions(Permissions::MANAGE_GUILD)
    .option(CommandOption::Integer(NumberCommandOptionData {
        autocomplete: false,
        choices: vec![],
        description: "The Season's id".to_string(),
        description_localizations: None,
        max_value: None,
        min_value: Some(CommandOptionValue::Integer(1)),
        name: "season_id".to_string(),
        name_localizations: None,
        required: true,
    }))
    .option(CommandOption::String(ChoiceCommandOptionData {
        autocomplete: false,
        choices: possible_states,
        description: "Set a season's state".to_string(),
        description_localizations: None,
        max_length: None,
        min_length: None,
        name: "new_state".to_string(),
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

    let add_player_to_bracket = CommandBuilder::new(
        ADD_PLAYER_TO_BRACKET_CMD.to_string(),
        "Add a player to a bracket".to_string(),
        CommandType::ChatInput,
    )
    .default_member_permissions(Permissions::MANAGE_GUILD)
    .option(CommandOption::User(BaseCommandOptionData {
        description: format!("The player. Add with /{}", ADD_PLAYER_TO_BRACKET_CMD),
        description_localizations: None,
        name: "user".to_string(),
        name_localizations: None,
        required: true,
    }))
    .option(CommandOption::String(ChoiceCommandOptionData {
        autocomplete: true,
        choices: vec![],
        description: "The bracket to add to".to_string(),
        description_localizations: None,
        max_length: None,
        min_length: None,
        name: "bracket".to_string(),
        name_localizations: None,
        required: true,
    }))
    .build();

    let finish_bracket = CommandBuilder::new(
        FINISH_BRACKET_CMD.to_string(),
        "Finish bracket".to_string(),
        CommandType::ChatInput,
    )
    .default_member_permissions(Permissions::MANAGE_GUILD)
    .option(CommandOption::Integer(NumberCommandOptionData {
        autocomplete: false,
        choices: vec![],
        description: "Bracket ID".to_string(),
        description_localizations: None,
        max_value: None,
        min_value: Some(CommandOptionValue::Integer(1)),
        name: "bracket_id".to_string(),
        name_localizations: None,
        required: true,
    }))
    .build();

    let create_player = CommandBuilder::new(
        CREATE_PLAYER_CMD.to_string(),
        "Add a player".to_string(),
        CommandType::ChatInput,
    )
    .default_member_permissions(Permissions::MANAGE_GUILD)
    .option(CommandOption::User(BaseCommandOptionData {
        description: "user".to_string(),
        description_localizations: None,
        name: "user".to_string(),
        name_localizations: None,
        required: true,
    }))
    .option(CommandOption::String(ChoiceCommandOptionData {
        autocomplete: false,
        choices: vec![],
        description: "RaceTime.gg username".to_string(),
        description_localizations: None,
        max_length: None,
        min_length: None,
        name: "rtgg_username".to_string(),
        name_localizations: None,
        required: true,
    }))
    .option(CommandOption::String(ChoiceCommandOptionData {
        autocomplete: false,
        choices: vec![],
        description: "Twitch username".to_string(),
        description_localizations: None,
        max_length: None,
        min_length: None,
        name: "twitch_username".to_string(),
        name_localizations: None,
        required: true,
    }))
    .option(CommandOption::String(ChoiceCommandOptionData {
        autocomplete: false,
        choices: vec![],
        description: "Name (if different than discord name)".to_string(),
        description_localizations: None,
        max_length: None,
        min_length: None,
        name: "name".to_string(),
        name_localizations: None,
        required: false,
    }))
    .build();

    let hours: Vec<CommandOptionChoice> = (1..=12)
        .map(|s| CommandOptionChoice::Int {
            name: format!("{}", s),
            name_localizations: None,
            value: s,
        })
        .collect();
    let ampm_opts = vec![
        CommandOptionChoice::String {
            name: "AM".to_string(),
            name_localizations: None,
            value: "AM".to_string(),
        },
        CommandOptionChoice::String {
            name: "PM".to_string(),
            name_localizations: None,
            value: "PM".to_string(),
        },
    ];

    let schedule_race = CommandBuilder::new(
        SCHEDULE_RACE_CMD.to_string(),
        "Schedule your next race (all times in US/Eastern)".to_string(),
        CommandType::ChatInput,
    )
    .option(CommandOption::Integer(NumberCommandOptionData {
        autocomplete: false,
        choices: hours.clone(),
        description: "Hour".to_string(),
        description_localizations: None,
        max_value: None,
        min_value: None,
        name: "hour".to_string(),
        name_localizations: None,
        required: true,
    }))
    .option(CommandOption::Integer(NumberCommandOptionData {
        autocomplete: false,
        choices: vec![],
        description: "Minute (1-59 to avoid noon/midnight confusion)".to_string(),
        description_localizations: None,
        max_value: Some(CommandOptionValue::Integer(59)),
        min_value: Some(CommandOptionValue::Integer(1)),
        name: "minute".to_string(),
        name_localizations: None,
        required: true,
    }))
    .option(CommandOption::String(ChoiceCommandOptionData {
        autocomplete: false,
        choices: ampm_opts.clone(),
        description: "AM/PM".to_string(),
        description_localizations: None,
        max_length: None,
        min_length: None,
        name: "am_pm".to_string(),
        name_localizations: None,
        required: true,
    }))
    .option(CommandOption::String(ChoiceCommandOptionData {
        autocomplete: true,
        choices: vec![],
        description: "Day (yyyy/mm/dd format) (wait for suggestions)".to_string(),
        description_localizations: None,
        max_length: None,
        min_length: None,
        name: "day".to_string(),
        name_localizations: None,
        required: true,
    }))
    .build();

    let report_race = CommandBuilder::new(
        REPORT_RACE_CMD.to_string(),
        "Report race".to_string(),
        CommandType::ChatInput,
    )
    .default_member_permissions(Permissions::MANAGE_GUILD)
    .option(CommandOption::Integer(NumberCommandOptionData {
        autocomplete: false,
        choices: vec![],
        description: "Race id".to_string(),
        description_localizations: None,
        max_value: None,
        min_value: None,
        name: "race_id".to_string(),
        name_localizations: None,
        required: true,
    }))
    .option(CommandOption::String(ChoiceCommandOptionData {
        autocomplete: false,
        choices: vec![],
        description: r#"Player 1 result ("forfeit" if forfeit, h:mm:ss otherwise)"#.to_string(),
        description_localizations: None,
        max_length: None,
        min_length: None,
        name: "p1_result".to_string(),
        name_localizations: None,
        required: true,
    }))
    .option(CommandOption::String(ChoiceCommandOptionData {
        autocomplete: false,
        choices: vec![],
        description: r#"Player 2 result ("forfeit" if forfeit, h:mm:ss otherwise)"#.to_string(),
        description_localizations: None,
        max_length: None,
        min_length: None,
        name: "p2_result".to_string(),
        name_localizations: None,
        required: true,
    }))
    .option(CommandOption::String(ChoiceCommandOptionData {
        autocomplete: false,
        choices: vec![],
        description: "Racetime url (if any)".to_string(),
        description_localizations: None,
        max_length: None,
        min_length: None,
        name: "racetime_url".to_string(),
        name_localizations: None,
        required: false,
    }))
    .build();

    let update_finished_race = CommandBuilder::new(
        UPDATE_FINISHED_RACE_CMD.to_string(),
        "Report race".to_string(),
        CommandType::ChatInput,
    )
    .default_member_permissions(Permissions::MANAGE_GUILD)
    .option(CommandOption::Integer(NumberCommandOptionData {
        autocomplete: false,
        choices: vec![],
        description: "Race id".to_string(),
        description_localizations: None,
        max_value: None,
        min_value: None,
        name: "race_id".to_string(),
        name_localizations: None,
        required: true,
    }))
    .option(CommandOption::String(ChoiceCommandOptionData {
        autocomplete: false,
        choices: vec![],
        description: r#"Player 1 result ("forfeit" if forfeit, h:mm:ss otherwise)"#.to_string(),
        description_localizations: None,
        max_length: None,
        min_length: None,
        name: "p1_result".to_string(),
        name_localizations: None,
        required: true,
    }))
    .option(CommandOption::String(ChoiceCommandOptionData {
        autocomplete: false,
        choices: vec![],
        description: r#"Player 2 result ("forfeit" if forfeit, h:mm:ss otherwise)"#.to_string(),
        description_localizations: None,
        max_length: None,
        min_length: None,
        name: "p2_result".to_string(),
        name_localizations: None,
        required: true,
    }))
    .option(CommandOption::String(ChoiceCommandOptionData {
        autocomplete: false,
        choices: vec![],
        description: "Racetime url (if any)".to_string(),
        description_localizations: None,
        max_length: None,
        min_length: None,
        name: "racetime_url".to_string(),
        name_localizations: None,
        required: false,
    }))
    .build();

    let generate_pairings = CommandBuilder::new(
        GENERATE_PAIRINGS_CMD.to_string(),
        "Generate next round pairings for a bracket".to_string(),
        CommandType::ChatInput,
    )
    .default_member_permissions(Permissions::MANAGE_GUILD)
    .option(CommandOption::Integer(NumberCommandOptionData {
        autocomplete: false,
        choices: vec![],
        description: "Bracket ID".to_string(),
        description_localizations: None,
        max_value: None,
        min_value: None,
        name: "bracket_id".to_string(),
        name_localizations: None,
        required: true,
    }))
    .build();

    let submit_qualifier = CommandBuilder::new(
        SUBMIT_QUALIFIER_CMD.to_string(),
        "Submit a time for qualification",
        CommandType::ChatInput,
    )
    .option(CommandOption::String(ChoiceCommandOptionData {
        autocomplete: false,
        choices: vec![],
        description: "Time in h:mm:ss format (e.g. 1:25:40)".to_string(),
        description_localizations: None,
        max_length: Some(8),
        min_length: Some(7),
        name: "qualifier_time".to_string(),
        name_localizations: None,
        required: true,
    }))
    .option(CommandOption::String(ChoiceCommandOptionData {
        autocomplete: false,
        choices: vec![],
        description: "Link to a VoD of the run".to_string(),
        description_localizations: None,
        max_length: None,
        min_length: None,
        name: "vod".to_string(),
        name_localizations: None,
        required: true,
    }))
    .build();

    let update_user_info = CommandBuilder::new(
        UPDATE_USER_INFO_CMD.to_string(),
        "Update your info (twitch, racetime, etc)",
        CommandType::ChatInput,
    )
    .option(CommandOption::String(ChoiceCommandOptionData {
        autocomplete: false,
        choices: vec![],
        description: "Set a new nickname for display".to_string(),
        description_localizations: None,
        max_length: None,
        min_length: None,
        name: "nickname".to_string(),
        name_localizations: None,
        required: false,
    }))
    .option(CommandOption::String(ChoiceCommandOptionData {
        autocomplete: false,
        choices: vec![],
        description: "Your twitch username as it appears in your stream's URL".to_string(),
        description_localizations: None,
        max_length: None,
        min_length: None,
        name: "twitch".to_string(),
        name_localizations: None,
        required: false,
    }))
    .option(CommandOption::String(ChoiceCommandOptionData {
        autocomplete: false,
        choices: vec![],
        description: "Your RaceTime.gg username, with discriminator (e.g. FoxLisk#8582)"
            .to_string(),
        description_localizations: None,
        max_length: None,
        min_length: None,
        name: "racetime".to_string(),
        name_localizations: None,
        required: false,
    }))
    .build();

    let check_user_info = CommandBuilder::new(
        CHECK_USER_INFO_CMD.to_string(),
        "Check your info (twitch, racetime, etc)",
        CommandType::ChatInput,
    )
    .build();

    let reschedule_race = CommandBuilder::new(
        RESCHEDULE_RACE_CMD.to_string(),
        "Reschedule someone else's race".to_string(),
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
    .option(CommandOption::Integer(NumberCommandOptionData {
        autocomplete: false,
        choices: hours,
        description: "Hour".to_string(),
        description_localizations: None,
        max_value: None,
        min_value: None,
        name: "hour".to_string(),
        name_localizations: None,
        required: true,
    }))
    .option(CommandOption::Integer(NumberCommandOptionData {
        autocomplete: false,
        choices: vec![],
        description: "Minute (1-59 to avoid noon/midnight confusion)".to_string(),
        description_localizations: None,
        max_value: Some(CommandOptionValue::Integer(59)),
        min_value: Some(CommandOptionValue::Integer(1)),
        name: "minute".to_string(),
        name_localizations: None,
        required: true,
    }))
    .option(CommandOption::String(ChoiceCommandOptionData {
        autocomplete: false,
        choices: ampm_opts,
        description: "AM/PM".to_string(),
        description_localizations: None,
        max_length: None,
        min_length: None,
        name: "am_pm".to_string(),
        name_localizations: None,
        required: true,
    }))
    .option(CommandOption::String(ChoiceCommandOptionData {
        autocomplete: true,
        choices: vec![],
        description: "Day (yyyy/mm/dd format) (wait for suggestions)".to_string(),
        description_localizations: None,
        max_length: None,
        min_length: None,
        name: "day".to_string(),
        name_localizations: None,
        required: true,
    }))
    .build();

    vec![
        create_race,
        cancel_race,
        create_season,
        set_season_state,
        create_bracket,
        finish_bracket,
        create_player,
        add_player_to_bracket,
        schedule_race,
        report_race,
        generate_pairings,
        reschedule_race,
        update_finished_race,
        submit_qualifier,
        update_user_info,
        check_user_info,
    ]
}
