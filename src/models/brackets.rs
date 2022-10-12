use diesel::prelude::{Insertable, Queryable};
use rocket::serde::json::serde_json;
use crate::models::season::Season;
use crate::save_fn;
use crate::schema::brackets;
use diesel::{SqliteConnection, RunQueryDsl};



#[derive(Queryable, Debug)]
#[allow(unused)]
pub struct Bracket {
    pub id: i32,
    name: String,
    season_id: i32,
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
    pub fn new<S: Into<String>>(season: &Season, name: S) -> Self {
        // ... okay this would be FINE to .unwrap(), but rules are rules
        Self {
            season_id: season.id,
            name: name.into(),
            state: serde_json::to_string(&BracketState::Unstarted).unwrap_or("Unknown".to_string()),
            current_round: None
        }
    }

    save_fn!(brackets::table, Bracket);
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