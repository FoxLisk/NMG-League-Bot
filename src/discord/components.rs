use twilight_model::channel::message::component::ActionRow;
use twilight_model::channel::message::Component;

pub fn action_row(components: Vec<Component>) -> Vec<Component> {
    vec![Component::ActionRow(ActionRow {
        id: None,
        components,
    })]
}
