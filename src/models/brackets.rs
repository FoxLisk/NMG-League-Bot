use diesel::prelude::Insertable;
use rocket::serde::json::serde_json;
use crate::schema::brackets;


#[derive(Queryable)]
#[allow(unused)]
pub struct Bracket {
    pub id: i32,
    season_id: i32,
    name: String,
    state: String,
    current_round: Option<i32>
}

#[derive(serde::Serialize, serde::Deserialize)]
enum BracketState {
    Unstarted,
    Started,
    Finished
}

#[derive(Insertable)]
#[diesel(table_name=brackets)]
pub struct NewBracket {
    season_id: i32,
    name: String,
    state: String,
    current_round: Option<i32>
}

impl NewBracket {
    fn new(bracket: &Bracket, name: String) -> Self {
        // ... okay this would be FINE to .unwrap(), but rules are rules
        Self {
            season_id: bracket.id,
            name,
            state: serde_json::to_string(&BracketState::Unstarted).unwrap_or("Unknown".to_string()),
            current_round: None
        }
    }
}

#[cfg(test)]
mod tests {
    use rocket::serde::json::serde_json;
    use crate::models::brackets::BracketState;

    #[test]
    fn test_serialize() {
        assert_eq!(r#""Unstarted""#.to_string(), serde_json::to_string(&BracketState::Unstarted).unwrap());
    }
}