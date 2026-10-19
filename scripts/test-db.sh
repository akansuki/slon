#!/usr/bin/env bash
# Starts a throwaway PostgreSQL cluster for the pg-core integration tests.
#
#   scripts/test-db.sh start   -> prints the env vars to export
#   scripts/test-db.sh stop
#
# The cluster is thrown away: its "secret" password and certificates are local test fixtures,
# never anything real.
#
# Needs the PostgreSQL server binaries (initdb, pg_ctl). When run as root it drops to
# the `postgres` user, since the server refuses to run as root.
set -euo pipefail

PGBIN="${PGBIN:-$(ls -d /usr/lib/postgresql/*/bin 2>/dev/null | sort -V | tail -1)}"
DIR="${SLON_TEST_DIR:-/tmp/slon-test-db}"
PORT="${SLON_TEST_PORT:-54329}"

as_pg() {
  if [ "$(id -u)" = 0 ]; then runuser -u postgres -- "$@"; else "$@"; fi
}

# A private CA, a server certificate it signed (valid for the name "localhost" only, on
# purpose: connecting by IP must fail verify-full), and an unrelated CA for the failure case.
gen_certs() {
  command -v openssl >/dev/null || return 0
  as_pg sh -c "cd '$DIR' &&
    openssl req -x509 -newkey rsa:2048 -nodes -days 30 -subj '/CN=slon test CA' -keyout ca.key -out ca.crt 2>/dev/null &&
    openssl req -x509 -newkey rsa:2048 -nodes -days 30 -subj '/CN=some other CA' -keyout other-ca.key -out other-ca.crt 2>/dev/null &&
    openssl req -newkey rsa:2048 -nodes -subj '/CN=localhost' -keyout server.key -out server.csr 2>/dev/null &&
    printf 'subjectAltName=DNS:localhost\nextendedKeyUsage=serverAuth\n' > san.ext &&
    openssl x509 -req -in server.csr -CA ca.crt -CAkey ca.key -CAcreateserial -days 30 -extfile san.ext -out server.crt 2>/dev/null &&
    cp server.crt data/server.crt && cp server.key data/server.key && chmod 600 data/server.key"
}

case "${1:-}" in
  start)
    if [ ! -d "$DIR/data" ]; then
      mkdir -p "$DIR"
      [ "$(id -u)" = 0 ] && chown postgres "$DIR"
      echo "secret" > "$DIR/pw"; [ "$(id -u)" = 0 ] && chown postgres "$DIR/pw"
      as_pg "$PGBIN/initdb" -D "$DIR/data" -U postgres --auth=scram-sha-256 --pwfile="$DIR/pw" >/dev/null
      if command -v openssl >/dev/null; then
        echo "ssl = on" | as_pg tee -a "$DIR/data/postgresql.conf" >/dev/null
      fi
    fi
    fresh_certs=0
    if [ ! -f "$DIR/ca.crt" ]; then gen_certs; fresh_certs=1; fi
    if ! as_pg "$PGBIN/pg_ctl" -D "$DIR/data" status >/dev/null 2>&1; then
      as_pg "$PGBIN/pg_ctl" -D "$DIR/data" -o "-p $PORT -k $DIR -c listen_addresses=127.0.0.1" -l "$DIR/log" -w start >/dev/null
    elif [ "$fresh_certs" = 1 ]; then
      as_pg "$PGBIN/pg_ctl" -D "$DIR/data" reload >/dev/null
    fi
    echo "export SLON_TEST_HOST=127.0.0.1 SLON_TEST_PORT=$PORT SLON_TEST_USER=postgres SLON_TEST_PASSWORD=secret SLON_TEST_DB=postgres SLON_TEST_CA=$DIR/ca.crt SLON_TEST_OTHER_CA=$DIR/other-ca.crt"
    ;;
  stop)
    as_pg "$PGBIN/pg_ctl" -D "$DIR/data" -m fast stop >/dev/null || true
    ;;
  *) echo "usage: $0 start|stop" >&2; exit 2 ;;
esac
