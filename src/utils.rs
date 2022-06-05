use serenity::model::interactions::{InteractionResponseType};
use serenity::model::interactions::application_command::ApplicationCommandInteraction;
use serenity::http::Http;

pub(crate) async fn send_response<T: Into<String>>(http: impl AsRef<Http>, interaction: ApplicationCommandInteraction, message: T) -> Result<(), String>{
    interaction
        .create_interaction_response(&http, |resp| {
            resp.kind(InteractionResponseType::ChannelMessageWithSource)
                .interaction_response_data(|data| data.content(message.into()))
        })
        .await
        .map_err(|e| e.to_string())
}