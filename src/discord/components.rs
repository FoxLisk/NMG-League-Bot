use twilight_model::channel::message::Component;
use twilight_model::channel::message::component::ActionRow;

pub fn action_row(components: Vec<Component>) -> Vec<Component> {
    vec![Component::ActionRow(ActionRow { components })]
}
