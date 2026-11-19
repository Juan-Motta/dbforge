//! Creating databases, roles and users on a connection's server, and
//! dropping a database, from the explorer's menus, as the former web UI's
//! `features/admin`. [`new_object`] holds what each engine asks for.

mod dialogs;
pub mod new_object;

pub use dialogs::{AdminEvent, DropDatabaseDialog, NewObjectDialog};
pub use new_object::NewObjectKind;
