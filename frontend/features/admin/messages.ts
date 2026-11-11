import { defineMessages } from '../../lib/i18n/messages'

export const messages = defineMessages({
  en: {
    menuDatabase: 'Database',
    menuRole: 'Role',
    menuUser: 'User',
    titleDatabase: (connection: string) => `New database on ${connection}`,
    titleRole: (connection: string) => `New role on ${connection}`,
    titleUser: (connection: string) => `New user on ${connection}`,
    createDatabase: 'Create database',
    createRole: 'Create role',
    createUser: 'Create user',
    creating: 'Creating…',
    cancel: 'Cancel',
    name: 'Name',
    password: 'Password',
    host: 'Host',
    firstCollection: 'First collection',
    database: 'Database',
    noteMongoDatabase:
      'MongoDB keeps a database once it holds a collection, so it is created with its first one.',
    noteDatabase: 'An empty database, owned by this connection’s user.',
    noteServerRole: 'A server role, without permissions. Grant them in the console.',
    noteRole: 'A role without privileges. Grant them in the console.',
    notePostgresUser: 'A role that can sign in, without privileges. Grant them in the console.',
    noteSqlServerUser:
      'A login with SQL Server authentication, without permissions. Grant them in the console.',
    noteRedisUser: 'An ACL user that is on, without permissions. Grant them with ACL SETUSER in the console.',
    noteUser: 'A user without privileges. Grant them in the console.',
    dropTitle: (database: string) => `Drop database ${database}?`,
    dropNote: (database: string, connection: string) =>
      `This deletes ${database} and everything in it from ${connection}’s server. It can’t be undone.`,
    dropConfirm: (database: string) => `Type ${database} to confirm`,
    dropDatabase: 'Drop database',
    dropping: 'Dropping…',
  },
  es: {
    menuDatabase: 'Base de datos',
    menuRole: 'Rol',
    menuUser: 'Usuario',
    titleDatabase: (connection: string) => `Nueva base de datos en ${connection}`,
    titleRole: (connection: string) => `Nuevo rol en ${connection}`,
    titleUser: (connection: string) => `Nuevo usuario en ${connection}`,
    createDatabase: 'Crear base de datos',
    createRole: 'Crear rol',
    createUser: 'Crear usuario',
    creating: 'Creando…',
    cancel: 'Cancelar',
    name: 'Nombre',
    password: 'Contraseña',
    host: 'Host',
    firstCollection: 'Primera colección',
    database: 'Base de datos',
    noteMongoDatabase:
      'MongoDB conserva una base de datos solo cuando tiene una colección, así que se crea con la primera.',
    noteDatabase: 'Una base de datos vacía, propiedad del usuario de esta conexión.',
    noteServerRole: 'Un rol de servidor, sin permisos. Otórgalos en la consola.',
    noteRole: 'Un rol sin privilegios. Otórgalos en la consola.',
    notePostgresUser: 'Un rol que puede iniciar sesión, sin privilegios. Otórgalos en la consola.',
    noteSqlServerUser:
      'Un inicio de sesión con autenticación de SQL Server, sin permisos. Otórgalos en la consola.',
    noteRedisUser: 'Un usuario ACL activo, sin permisos. Otórgalos con ACL SETUSER en la consola.',
    noteUser: 'Un usuario sin privilegios. Otórgalos en la consola.',
    dropTitle: (database: string) => `¿Eliminar la base de datos ${database}?`,
    dropNote: (database: string, connection: string) =>
      `Esto borra ${database} y todo lo que contiene del servidor de ${connection}. No se puede deshacer.`,
    dropConfirm: (database: string) => `Escribe ${database} para confirmar`,
    dropDatabase: 'Eliminar base de datos',
    dropping: 'Eliminando…',
  },
})

export type AdminMessages = typeof messages.en
