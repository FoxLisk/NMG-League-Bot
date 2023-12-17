pub mod asyncs;
pub mod bracket_race_infos;
pub mod bracket_races;
pub mod bracket_rounds;
pub mod brackets;
pub mod player;
pub mod player_bracket_entries;
pub mod qualifer_submission;
pub mod season;

// TODO: should this be a derive macro?
/// creates a function named `save()` that takes a &SqliteConnection
#[macro_export]
macro_rules! save_fn {
    ($table:expr, $output:ty) => {
        pub fn save(&self, cxn: &mut diesel::SqliteConnection) -> diesel::QueryResult<$output> {
            use diesel::RunQueryDsl;
            diesel::insert_into($table).values(self).get_result(cxn)
        }
    };
}

#[macro_export]
macro_rules! update_fn {
    () => {
        pub fn update(&self, conn: &mut diesel::SqliteConnection) -> diesel::QueryResult<usize> {
            diesel::update(self).set(self).execute(conn)
        }
    };
}

#[macro_export]
macro_rules! delete_fn {
    ($table:expr) => {
        pub fn delete(self, conn: &mut diesel::SqliteConnection) -> diesel::QueryResult<usize> {
            diesel::delete($table.find(self.id)).execute(conn)
        }
    };
}
