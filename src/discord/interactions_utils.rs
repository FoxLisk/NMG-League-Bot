use log::warn;
use nmg_league_bot::ApplicationCommandOptionError;
use twilight_model::application::command::CommandOptionChoice;
use twilight_model::application::interaction::application_command::{
    CommandDataOption, CommandOptionValue,
};
use twilight_model::application::interaction::{Interaction, InteractionData};
use twilight_model::channel::message::component::{Button, ButtonStyle};
use twilight_model::channel::message::{AllowedMentions, Component, MessageFlags};
use twilight_model::http::interaction::{
    InteractionResponse, InteractionResponseData, InteractionResponseType,
};

/// InteractionResponseData with just content + no allowed mentions
pub fn plain_interaction_data<S: Into<String>>(content: S) -> InteractionResponseData {
    InteractionResponseData {
        content: Some(content.into()),
        allowed_mentions: Some(AllowedMentions::default()),
        ..Default::default()
    }
}

/// Creates a basic interaction response: new message, plain content with no allowed mentions.
pub fn plain_interaction_response<S: Into<String>>(content: S) -> InteractionResponse {
    InteractionResponse {
        kind: InteractionResponseType::ChannelMessageWithSource,
        data: Some(plain_interaction_data(content)),
    }
}

pub fn update_resp_to_plain_content<S: Into<String>>(content: S) -> InteractionResponse {
    InteractionResponse {
        kind: InteractionResponseType::UpdateMessage,
        data: Some(InteractionResponseData {
            components: Some(vec![]),
            content: Some(content.into()),
            ..Default::default()
        }),
    }
}

pub fn plain_ephemeral_response<S: Into<String>>(content: S) -> InteractionResponse {
    let mut data = plain_interaction_data(content);
    data.flags = Some(MessageFlags::EPHEMERAL);
    InteractionResponse {
        kind: InteractionResponseType::ChannelMessageWithSource,
        data: Some(data),
    }
}

pub fn button_component<S1: Into<String>, S2: Into<String>>(
    label: S1,
    custom_id: S2,
    style: ButtonStyle,
) -> Component {
    Component::Button(Button {
        custom_id: Some(custom_id.into()),
        disabled: false,
        emoji: None,
        label: Some(label.into()),
        style,
        url: None,
        id: None,
        sku_id: None,
    })
}

pub fn interaction_to_custom_id(i: &Interaction) -> Option<&str> {
    match i.data.as_ref()? {
        InteractionData::ApplicationCommand(_ac) => None,
        InteractionData::MessageComponent(mc) => Some(&mc.custom_id),
        InteractionData::ModalSubmit(ms) => Some(&ms.custom_id),
        _ => {
            warn!("Unhandled InteractionData type: {:?}", i);
            None
        }
    }
}

pub fn autocomplete_result(options: Vec<CommandOptionChoice>) -> InteractionResponse {
    InteractionResponse {
        kind: InteractionResponseType::ApplicationCommandAutocompleteResult,
        data: Some(InteractionResponseData {
            allowed_mentions: None,
            attachments: None,
            choices: Some(options),
            components: None,
            content: None,
            custom_id: None,
            embeds: None,
            flags: None,
            title: None,
            tts: None,
            poll: None,
        }),
    }
}

/// returns, effectively, the subcommand option itself; unbundled into a (name, [options]) tuple
pub fn get_subcommand_options(
    options: Vec<CommandDataOption>,
) -> Result<(String, Vec<CommandDataOption>), ApplicationCommandOptionError> {
    if options.len() != 1 {
        warn!("`get_subcommand` called on a list of more than one option, which is probably not correct");
    }
    for option in options {
        if let CommandOptionValue::SubCommand(sc) = option.value {
            return Ok((option.name, sc));
        }
    }
    Err(ApplicationCommandOptionError::NoSubcommand)
}
