use super::Messages;

pub(super) const MESSAGES: Messages = Messages {
    connections: "Conexiones",
    new_connection: "Nueva conexión",
    refresh: "Actualizar",
    filter_placeholder: "Filtrar objetos…",
    no_connections: "Aún no hay conexiones.",
    collapse_all: "Contraer todo",
    no_objects: "Sin objetos",
    no_matches: |query| format!("Ningún objeto coincide con “{query}”."),
    tables: "Tablas",
    views: "Vistas",
    copy_name: "Copiar nombre",
    refresh_connection: "Actualizar conexión",
    resize_explorer: "Cambiar el ancho del explorador",
    no_connection: "Ninguna conexión seleccionada",
    search_everywhere: "Buscar en todo…",
    settings: "Configuración",
    new_query: "Nueva consulta",
    empty_before: "Haz doble clic en una tabla para ver sus filas, o pulsa ",
    empty_after: ".",
    set_aside_title: "No se pudieron leer algunos ajustes",
    set_aside_note: "Estos archivos estaban dañados o no tenían el formato esperado. DBForge los conservó en su \
                     lugar, con otro nombre, y arrancó sin ellos:",
    set_aside_ok: "Entendido",
};
