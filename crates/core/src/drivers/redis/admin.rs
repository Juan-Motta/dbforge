//! Users created from a connection's menu (Redis has no databases or roles to create).

use redis::Value;

use super::connect;
use crate::connections::model::Connection;
use crate::error::{AppError, Result};

/// Creates an ACL user that is on and signs in with `secret`, without
/// permissions. `ACL SETUSER` also changes existing users, so an existing
/// name is refused instead.
pub async fn create_user(connection: &Connection, password: Option<&str>, name: &str, secret: &str) -> Result<()> {
    let mut conn = connect(connection, password, None, None).await?;
    let existing: Value = redis::cmd("ACL").arg("GETUSER").arg(name).query_async(&mut conn).await?;
    if existing != Value::Nil {
        return Err(AppError::Invalid(format!("User {name} already exists.")));
    }
    redis::cmd("ACL").arg("SETUSER").arg(name).arg("on").arg(format!(">{secret}")).query_async::<()>(&mut conn).await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::super::tests::{PASSWORD, local};
    use super::*;
    use crate::drivers::{NewObject, create_object};

    #[tokio::test]
    #[ignore = "needs a local Redis on port 56379"]
    async fn creates_a_user_that_can_sign_in_and_never_changes_an_existing_one() {
        let mut conn = connect(&local(), PASSWORD, None, None).await.unwrap();
        redis::cmd("ACL").arg("DELUSER").arg("admin_test_user").query_async::<()>(&mut conn).await.unwrap();

        let user = |secret: &str| NewObject::User {
            name: "admin_test_user".into(),
            password: secret.into(),
            host: None,
            database: None,
        };
        create_object(&local(), PASSWORD, &user("it's \\ secret")).await.unwrap();
        // The user signs in with exactly the password it was given.
        let mut as_user = local();
        as_user.username = "admin_test_user".into();
        connect(&as_user, Some("it's \\ secret"), None, None).await.unwrap();

        let again = create_object(&local(), PASSWORD, &user("other")).await.unwrap_err();
        assert_eq!(again.to_string(), "User admin_test_user already exists.");
        connect(&as_user, Some("it's \\ secret"), None, None).await.unwrap();

        let role = NewObject::Role { name: "r".into(), database: None };
        let refused = create_object(&local(), PASSWORD, &role).await.unwrap_err();
        assert_eq!(refused.to_string(), "This connection can't create a role.");
        redis::cmd("ACL").arg("DELUSER").arg("admin_test_user").query_async::<()>(&mut conn).await.unwrap();
    }
}
