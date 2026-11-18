//! A table's structure, read from the core once per tab and again on
//! reload. The structure shown stays while the next one loads, so the
//! sections and their counts never blank out; a reply to an older request
//! is dropped.

use std::rc::Rc;

use dbforge_core::drivers::{TableRef, TableStructure};
use dbforge_core::explorer::table_structure;
use gpui::Context;

use crate::backend::Backend;

pub struct StructureSource {
    connection_id: String,
    table: TableRef,
    structure: Option<Rc<TableStructure>>,
    /// Why the last read failed; cleared by the next one that works.
    error: Option<String>,
    loading: bool,
    /// The newest read asked for; an older one arriving late is dropped.
    request: u64,
}

impl StructureSource {
    /// Reads `table`'s structure on `connection_id` at once.
    pub fn new(connection_id: String, table: TableRef, cx: &mut Context<Self>) -> Self {
        let mut source = Self { connection_id, table, structure: None, error: None, loading: false, request: 0 };
        source.reload(cx);
        source
    }

    /// The structure last read, kept while the next one loads.
    pub fn structure(&self) -> Option<&Rc<TableStructure>> {
        self.structure.as_ref()
    }

    /// Why the last read failed, once it has.
    pub fn error(&self) -> Option<&str> {
        if self.loading { None } else { self.error.as_deref() }
    }

    pub fn is_loading(&self) -> bool {
        self.loading
    }

    /// Reads the structure again; observers hear when it arrives.
    pub fn reload(&mut self, cx: &mut Context<Self>) {
        self.request += 1;
        let request = self.request;
        self.loading = true;
        let (connection, table) = (self.connection_id.clone(), self.table.clone());
        let reading = Backend::global(cx).spawn(cx, move |core| async move {
            table_structure(core.connections(), core.statement_log(), connection, table).await
        });
        cx.spawn(async move |this, cx| {
            let result = reading.await;
            let _ = this.update(cx, |this, cx| {
                if this.request != request {
                    return;
                }
                this.loading = false;
                match result {
                    Ok(structure) => {
                        this.structure = Some(Rc::new(structure));
                        this.error = None;
                    }
                    Err(error) => this.error = Some(error.to_string()),
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    /// Shows `structure` as if it had been read, for headless tests.
    #[cfg(test)]
    pub fn show(&mut self, structure: TableStructure, cx: &mut Context<Self>) {
        self.request += 1;
        self.loading = false;
        self.error = None;
        self.structure = Some(Rc::new(structure));
        cx.notify();
    }
}
