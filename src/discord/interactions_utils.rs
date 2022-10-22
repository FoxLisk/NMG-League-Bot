use twilight_model::application::component::button::ButtonStyle;
use twilight_model::application::component::{Button, Component};
use twilight_model::application::interaction::{Interaction, InteractionData};
use twilight_model::channel::message::allowed_mentions::AllowedMentionsBuilder;
use twilight_model::http::interaction::{
    InteractionResponse, InteractionResponseData, InteractionResponseType,
};

/// InteractionResponseData with just content + no allowed mentions
fn plain_interaction_data<S: Into<String>>(content: S) -> InteractionResponseData {
    InteractionResponseData {
        content: Some(content.into()),
        allowed_mentions: Some(AllowedMentionsBuilder::new().build()),
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
    })
}

pub fn interaction_to_custom_id(i: &Interaction) -> Option<&str> {
    match i.data.as_ref()? {
        InteractionData::ApplicationCommand(_ac) => None,
        InteractionData::MessageComponent(mc) => Some(&mc.custom_id),
        InteractionData::ModalSubmit(ms) => Some(&ms.custom_id),
        _ => {
            println!("Unhandled InteractionData type: {:?}", i);
            None
        }
    }
}
