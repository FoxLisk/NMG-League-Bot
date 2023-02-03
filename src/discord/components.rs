use twilight_model::application::component::{ActionRow, Component};

pub fn action_row(components: Vec<Component>) -> Vec<Component> {
    vec![Component::ActionRow(ActionRow { components })]
}
