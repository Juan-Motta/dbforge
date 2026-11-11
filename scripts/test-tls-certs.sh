#!/usr/bin/env bash
# Makes what the TLS container tests use, in the folder given (default
# ~/.cache/dbforge-test-certs, a folder Docker Desktop can mount; the tests read
# it from DBFORGE_TEST_CERTS): a test authority, a server certificate for
# "localhost" (not 127.0.0.1, so tests can tell a name check from a CA check),
# a client certificate for the user "certuser", and the init scripts that create
# that user. Then start the containers the tests name:
#
#   C=~/.cache/dbforge-test-certs
#   docker run -d --name dbforge-tls-postgres -p 55433:5432 -e POSTGRES_PASSWORD=dbforge \
#     -v $C:/certs:ro -v $C/pg-init:/docker-entrypoint-initdb.d:ro --entrypoint bash postgres:17 -c \
#     'install -o postgres -m 600 /certs/server.key /tmp/ && install -m 644 /certs/server.crt /certs/ca.crt /tmp/ &&
#      exec docker-entrypoint.sh postgres -c ssl=on -c ssl_cert_file=/tmp/server.crt -c ssl_key_file=/tmp/server.key -c ssl_ca_file=/tmp/ca.crt'
#   docker run -d --name dbforge-tls-mysql -p 53316:3306 -e MYSQL_ROOT_PASSWORD=dbforge \
#     -v $C:/certs:ro -v $C/mysql-init:/docker-entrypoint-initdb.d:ro --entrypoint bash mysql:8.4 -c \
#     'mkdir -p /tls && cp /certs/ca.crt /certs/server.* /tls/ && chown -R mysql /tls &&
#      exec docker-entrypoint.sh mysqld --ssl-ca=/tls/ca.crt --ssl-cert=/tls/server.crt --ssl-key=/tls/server.key'
#   docker run -d --name dbforge-tls-redis -p 56381:6380 -v $C:/certs:ro --entrypoint sh redis:7 -c \
#     'mkdir -p /tls && cp /certs/ca.crt /certs/server.* /tls/ && chown -R redis /tls &&
#      exec docker-entrypoint.sh redis-server --port 0 --tls-port 6380 --tls-cert-file /tls/server.crt
#      --tls-key-file /tls/server.key --tls-ca-cert-file /tls/ca.crt --tls-auth-clients yes --requirepass dbforge'
#   docker run -d --name dbforge-tls-mongo -p 57018:27017 -v $C:/certs:ro -e MONGO_INITDB_ROOT_USERNAME=root \
#     -e MONGO_INITDB_ROOT_PASSWORD=dbforge --entrypoint bash mongo:7 -c \
#     'mkdir -p /tls && cp /certs/ca.crt /certs/server.pem /tls/ && chown -R mongodb /tls &&
#      exec docker-entrypoint.sh mongod --tlsMode requireTLS --tlsCertificateKeyFile /tls/server.pem
#      --tlsCAFile /tls/ca.crt --tlsAllowConnectionsWithoutCertificates'
#   docker run -d --name dbforge-tls-mssql -p 51434:1433 -e ACCEPT_EULA=1 -e 'MSSQL_SA_PASSWORD=DbForge#2026' \
#     -v $C:/certs:ro --user root --entrypoint bash mcr.microsoft.com/azure-sql-edge -c \
#     'mkdir -p /tls /var/opt/mssql && cp /certs/server.* /tls/ &&
#      printf "[network]\ntlscert = /tls/server.crt\ntlskey = /tls/server.key\nforceencryption = 0\n" > /var/opt/mssql/mssql.conf &&
#      exec /opt/mssql/bin/permissions_check.sh /opt/mssql/bin/sqlservr'
set -euo pipefail

dir="${1:-$HOME/.cache/dbforge-test-certs}"
mkdir -p "$dir"
cd "$dir"

openssl req -x509 -newkey rsa:2048 -nodes -days 3650 -subj "/CN=DBForge Test CA" \
  -keyout ca.key -out ca.crt 2>/dev/null

sign() { # name, subject, extensions
  openssl req -newkey rsa:2048 -nodes -subj "$2" -keyout "$1.key" -out "$1.csr" 2>/dev/null
  printf '%s\n' "$3" > "$1.ext"
  openssl x509 -req -in "$1.csr" -CA ca.crt -CAkey ca.key -CAcreateserial -days 3650 \
    -extfile "$1.ext" -out "$1.crt" 2>/dev/null
  rm "$1.csr" "$1.ext"
}

sign server "/CN=localhost" "subjectAltName=DNS:localhost
extendedKeyUsage=serverAuth"
sign client "/CN=certuser" "extendedKeyUsage=clientAuth"

# MongoDB reads a certificate and its key from one file.
cat server.crt server.key > server.pem
cat client.crt client.key > client.pem
# The containers copy what they need as root; libpq refuses a client key others can read.
chmod 644 ./*.crt ./*.pem server.key
chmod 600 client.key ca.key

# certuser signs in with its certificate only.
mkdir -p pg-init mysql-init
cat > pg-init/tls.sh <<'SH'
#!/bin/bash
set -e
psql -v ON_ERROR_STOP=1 -U postgres -c "CREATE ROLE certuser LOGIN"
cat > "$PGDATA/pg_hba.conf" <<'HBA'
local all all trust
hostssl all certuser all cert
hostssl all all all scram-sha-256
hostnossl all all all scram-sha-256
HBA
SH
chmod 755 pg-init/tls.sh
cat > mysql-init/certuser.sql <<'SQL'
CREATE USER 'certuser'@'%' REQUIRE X509;
GRANT SELECT ON *.* TO 'certuser'@'%';
SQL

echo "$dir"
