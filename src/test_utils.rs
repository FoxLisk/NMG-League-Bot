use diesel::{Connection as _, SqliteConnection};
use oauth2::http::uri::Scheme;

use crate::{db::run_migrations, models::player::NewPlayer};

pub fn setup_db() -> Result<SqliteConnection, anyhow::Error> {
    let mut db = SqliteConnection::establish(":memory:")?;
    run_migrations(&mut db)?;
    Ok(db)
}

#[cfg(test)]
mod tests {
    use super::setup_db;
    use crate::models::player::NewPlayer;
    use diesel::dsl::count;
    use diesel::prelude::*;

    #[test]
    fn test_database_init() -> anyhow::Result<()> {
        let mut db = setup_db()?;
        NewPlayer::new("name", "1234", None, None, None).save(&mut db)?;
        let count = crate::schema::players::table
            .select(count(crate::schema::players::id))
            .get_result::<i64>(&mut db)?;
        assert_eq!(1, count);
        Ok(())
    }

    #[test]
    fn test_database_init_is_isolated() -> anyhow::Result<()> {
        // this is just a separate test to make sure that the player created in the previous test doesn't carry over

        let mut db = setup_db()?;
        NewPlayer::new("name", "1234", None, None, None).save(&mut db)?;
        let count = crate::schema::players::table
            .select(count(crate::schema::players::id))
            .get_result::<i64>(&mut db)?;
        assert_eq!(1, count);
        Ok(())
    }
}
