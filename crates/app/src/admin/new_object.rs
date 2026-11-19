//! What the New database / role / user dialog asks for on each engine, as
//! the former web UI's `newObject.ts`: its fields, when it is complete, the request
//! it sends and the texts it shows. Pure, so it is tested without a window.

use dbforge_core::connections::model::Engine;
use dbforge_core::drivers::NewObject;

use crate::i18n::AdminMessages;

/// What a connection's menu can create on its server.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum NewObjectKind {
    Database,
    Role,
    User,
}

/// What each engine's server can create, in menu order; none for a file.
pub fn new_objects(engine: Engine) -> &'static [NewObjectKind] {
    use NewObjectKind::*;
    match engine {
        Engine::Postgres | Engine::Mysql | Engine::Mariadb | Engine::Sqlserver | Engine::Mongodb => {
            &[Database, Role, User]
        }
        Engine::Cosmosdb => &[Database],
        Engine::Redis => &[User],
        Engine::Sqlite => &[],
    }
}

/// Whether a database's menu can drop it from its server.
pub fn drops_databases(engine: Engine) -> bool {
    !matches!(engine, Engine::Sqlite | Engine::Redis)
}

/// A field of the dialog beyond the name.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ExtraField {
    Password,
    Host,
    Collection,
    Database,
}

/// What the form holds.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FieldValues {
    pub name: String,
    pub password: String,
    pub host: String,
    pub collection: String,
    pub database: String,
}

impl FieldValues {
    pub fn get(&self, field: ExtraField) -> &str {
        match field {
            ExtraField::Password => &self.password,
            ExtraField::Host => &self.host,
            ExtraField::Collection => &self.collection,
            ExtraField::Database => &self.database,
        }
    }

    pub fn set(&mut self, field: ExtraField, value: String) {
        match field {
            ExtraField::Password => self.password = value,
            ExtraField::Host => self.host = value,
            ExtraField::Collection => self.collection = value,
            ExtraField::Database => self.database = value,
        }
    }
}

/// The fields `kind` needs on `engine`, in form order.
pub fn fields_of(engine: Engine, kind: NewObjectKind) -> Vec<ExtraField> {
    use ExtraField::*;
    let mongo = engine == Engine::Mongodb;
    match kind {
        NewObjectKind::Database if mongo => vec![Collection],
        NewObjectKind::Database => Vec::new(),
        NewObjectKind::Role if mongo => vec![Database],
        NewObjectKind::Role => Vec::new(),
        NewObjectKind::User if mongo => vec![Password, Database],
        NewObjectKind::User if matches!(engine, Engine::Mysql | Engine::Mariadb) => vec![Password, Host],
        NewObjectKind::User => vec![Password],
    }
}

/// What an optional field left empty means, shown as its placeholder.
pub fn placeholder(field: ExtraField) -> Option<&'static str> {
    match field {
        ExtraField::Host => Some("%"),
        ExtraField::Database => Some("admin"),
        ExtraField::Password | ExtraField::Collection => None,
    }
}

/// Every required field is filled: the name, a user's password, MongoDB's
/// first collection. A password of spaces counts; a name of spaces doesn't.
pub fn is_complete(values: &FieldValues, fields: &[ExtraField]) -> bool {
    !values.name.trim().is_empty()
        && fields.iter().all(|field| match field {
            ExtraField::Host | ExtraField::Database => true,
            ExtraField::Password => !values.password.is_empty(),
            ExtraField::Collection => !values.collection.trim().is_empty(),
        })
}

/// The request for `kind` from the form's values. Names are trimmed;
/// passwords never are; only the fields the engine uses are sent.
pub fn to_new_object(kind: NewObjectKind, fields: &[ExtraField], values: &FieldValues) -> NewObject {
    let name = values.name.trim().to_owned();
    let optional = |field: ExtraField| fields.contains(&field).then(|| values.get(field).trim().to_owned());
    match kind {
        NewObjectKind::Database => NewObject::Database { name, collection: optional(ExtraField::Collection) },
        NewObjectKind::Role => NewObject::Role { name, database: optional(ExtraField::Database) },
        NewObjectKind::User => NewObject::User {
            name,
            password: values.password.clone(),
            host: optional(ExtraField::Host),
            database: optional(ExtraField::Database),
        },
    }
}

/// The connection menu's item for `kind` under its New heading.
pub fn menu_label_of(kind: NewObjectKind, m: &AdminMessages) -> &'static str {
    match kind {
        NewObjectKind::Database => m.menu_database,
        NewObjectKind::Role => m.menu_role,
        NewObjectKind::User => m.menu_user,
    }
}

pub fn menu_icon_of(kind: NewObjectKind) -> &'static str {
    match kind {
        NewObjectKind::Database => "database",
        NewObjectKind::Role => "role",
        NewObjectKind::User => "user",
    }
}

/// The dialog's title for `kind` on `connection`.
pub fn title_of(kind: NewObjectKind, connection: &str, m: &AdminMessages) -> String {
    match kind {
        NewObjectKind::Database => (m.title_database)(connection),
        NewObjectKind::Role => (m.title_role)(connection),
        NewObjectKind::User => (m.title_user)(connection),
    }
}

/// The dialog's submit button for `kind`.
pub fn create_label_of(kind: NewObjectKind, m: &AdminMessages) -> &'static str {
    match kind {
        NewObjectKind::Database => m.create_database,
        NewObjectKind::Role => m.create_role,
        NewObjectKind::User => m.create_user,
    }
}

/// A field's label.
pub fn label_of(field: ExtraField, m: &AdminMessages) -> &'static str {
    match field {
        ExtraField::Password => m.password,
        ExtraField::Host => m.host,
        ExtraField::Collection => m.first_collection,
        ExtraField::Database => m.database,
    }
}

/// What creating `kind` on `engine` does, in one line under the form.
pub fn note_of(engine: Engine, kind: NewObjectKind, m: &AdminMessages) -> &'static str {
    match kind {
        NewObjectKind::Database if engine == Engine::Mongodb => m.note_mongo_database,
        NewObjectKind::Database => m.note_database,
        NewObjectKind::Role if engine == Engine::Sqlserver => m.note_server_role,
        NewObjectKind::Role => m.note_role,
        NewObjectKind::User => match engine {
            Engine::Postgres => m.note_postgres_user,
            Engine::Sqlserver => m.note_sql_server_user,
            Engine::Redis => m.note_redis_user,
            _ => m.note_user,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::i18n::Language;
    use ExtraField::{Collection, Host, Password};
    use NewObjectKind::*;

    fn en() -> &'static AdminMessages {
        &Language::En.messages().admin
    }

    #[test]
    fn asks_each_engine_only_for_what_it_needs() {
        assert_eq!(fields_of(Engine::Postgres, Database), []);
        assert_eq!(fields_of(Engine::Postgres, User), [Password]);
        assert_eq!(fields_of(Engine::Mysql, User), [Password, Host]);
        assert_eq!(fields_of(Engine::Mariadb, User), [Password, Host]);
        assert_eq!(fields_of(Engine::Mongodb, Database), [Collection]);
        assert_eq!(fields_of(Engine::Mongodb, Role), [ExtraField::Database]);
        assert_eq!(fields_of(Engine::Mongodb, User), [Password, ExtraField::Database]);
        assert_eq!(fields_of(Engine::Redis, User), [Password]);
    }

    #[test]
    fn offers_what_each_server_can_create_and_drop() {
        assert_eq!(new_objects(Engine::Postgres), [Database, Role, User]);
        assert_eq!(new_objects(Engine::Mongodb), [Database, Role, User]);
        assert_eq!(new_objects(Engine::Cosmosdb), [Database]);
        assert_eq!(new_objects(Engine::Redis), [User]);
        assert!(new_objects(Engine::Sqlite).is_empty());
        assert!(drops_databases(Engine::Sqlserver) && drops_databases(Engine::Cosmosdb));
        assert!(!drops_databases(Engine::Sqlite) && !drops_databases(Engine::Redis));
    }

    fn values(name: &str) -> FieldValues {
        FieldValues { name: name.into(), ..FieldValues::default() }
    }

    #[test]
    fn needs_a_name_a_user_s_password_and_mongodb_s_first_collection_not_the_optional_fields() {
        let user = fields_of(Engine::Mysql, User);
        assert!(!is_complete(&values(" "), &[]));
        assert!(is_complete(&values("shop"), &[]));
        assert!(!is_complete(&values("ann"), &user));
        assert!(is_complete(&FieldValues { password: " ".into(), ..values("ann") }, &user));
        assert!(!is_complete(&values("shop"), &fields_of(Engine::Mongodb, Database)));
    }

    #[test]
    fn trims_names_keeps_passwords_as_typed_and_sends_only_the_fields_the_engine_uses() {
        let typed = FieldValues {
            name: " ann ".into(),
            password: " p ".into(),
            host: " localhost ".into(),
            collection: "x".into(),
            database: " app ".into(),
        };
        // The core's request has no equality; its debug form shows every field.
        let sent = |kind, engine| format!("{:?}", to_new_object(kind, &fields_of(engine, kind), &typed));
        let expected = |object: NewObject| format!("{object:?}");
        assert_eq!(
            sent(User, Engine::Mysql),
            expected(NewObject::User {
                name: "ann".into(),
                password: " p ".into(),
                host: Some("localhost".into()),
                database: None
            })
        );
        assert_eq!(
            sent(Role, Engine::Mongodb),
            expected(NewObject::Role { name: "ann".into(), database: Some("app".into()) })
        );
        assert_eq!(
            sent(Database, Engine::Postgres),
            expected(NewObject::Database { name: "ann".into(), collection: None })
        );
    }

    #[test]
    fn writes_the_dialog_in_the_interface_language() {
        let es = &Language::Es.messages().admin;
        assert_eq!(title_of(User, "local", en()), "New user on local");
        assert_eq!(title_of(Database, "local", es), "Nueva base de datos en local");
        assert_eq!(note_of(Engine::Postgres, Role, en()), "A role without privileges. Grant them in the console.");
        assert_eq!(note_of(Engine::Postgres, Role, es), "Un rol sin privilegios. Otórgalos en la consola.");
        assert_eq!(note_of(Engine::Sqlserver, Role, en()), en().note_server_role);
        assert_eq!(note_of(Engine::Redis, User, en()), en().note_redis_user);
        assert_eq!(note_of(Engine::Mongodb, Database, en()), en().note_mongo_database);
        assert_eq!(create_label_of(Role, en()), "Create role");
        assert_eq!(placeholder(Host), Some("%"));
    }
}
