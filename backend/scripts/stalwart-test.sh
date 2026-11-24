#!/usr/bin/env bash
# A throwaway Stalwart for the JMAP tests against a real server (src/jmap/stalwart_tests.rs): starts the image in a
# container, finishes its bootstrap through the API, creates two users (alice, bob @example.com) and an OAuth client,
# and writes the variables the tests read to backend/.stalwart-test.env.
#
#   scripts/stalwart-test.sh            # start (or reuse) and provision
#   set -a; . ./.stalwart-test.env; set +a
#   cargo test stalwart -- --test-threads=1
#   scripts/stalwart-test.sh stop       # remove the container
#
# Stalwart advertises https://mail.example.com/... in its session (its hostname; the setting takes no port), which the
# tests send to the container instead (JMAP_TEST_REWRITE, read only by the tests).
#
# STALWART_IMAGE (default docker.io/stalwartlabs/stalwart:v1.2.0), STALWART_PORT (default 18080). podman or docker.
set -euo pipefail

IMAGE="${STALWART_IMAGE:-docker.io/stalwartlabs/stalwart:v1.2.0}"
PORT="${STALWART_PORT:-18080}"
NAME="caley-stalwart-test"
BASE="http://localhost:$PORT"
OUT="$(cd "$(dirname "$0")/.." && pwd)/.stalwart-test.env"
ENGINE="$(command -v podman || command -v docker)"

if [[ "${1:-}" == "stop" ]]; then
  "$ENGINE" rm -f "$NAME" >/dev/null
  rm -f "$OUT"
  exit 0
fi

if [[ -f "$OUT" ]] && "$ENGINE" container exists "$NAME" 2>/dev/null; then
  echo "already provisioned: $OUT"
  exit 0
fi

wait_http() {
  for _ in $(seq 1 60); do
    curl -s -o /dev/null "$BASE/.well-known/jmap" && return 0
    sleep 1
  done
  echo "Stalwart did not answer on $BASE" >&2
  exit 1
}

jmap() { # user:password, JSON body
  curl -sf -u "$1" "$BASE/jmap/" -H 'content-type: application/json' -d "$2"
}

json() { python3 -c "import json,sys; d=json.load(sys.stdin); print($1)"; }

"$ENGINE" rm -f "$NAME" >/dev/null 2>&1 || true
"$ENGINE" run -d --name "$NAME" -p "$PORT:8080" "$IMAGE" >/dev/null
wait_http

# bootstrap mode prints a temporary administrator once
BOOT_PASS=""
for _ in $(seq 1 30); do
  BOOT_PASS="$("$ENGINE" logs "$NAME" 2>&1 | sed -n 's/^ *password: //p' | head -1)"
  [[ -n "$BOOT_PASS" ]] && break
  sleep 1
done
[[ -n "$BOOT_PASS" ]] || { echo "no bootstrap password in the logs" >&2; exit 1; }

ADMIN="$(jmap "admin:$BOOT_PASS" '{"using":["urn:ietf:params:jmap:core","urn:stalwart:jmap"],"methodCalls":[["x:Bootstrap/set",{"update":{"singleton":{"defaultHostname":"mail.example.com","defaultDomain":"example.com","requestTlsCertificate":false,"generateDkimKeys":false}}},"c"]]}' \
  | json 'd["methodResponses"][0][1]["updated"]["singleton"]["username"]+":"+d["methodResponses"][0][1]["updated"]["singleton"]["secret"]')"
"$ENGINE" restart "$NAME" >/dev/null
wait_http
for _ in $(seq 1 30); do
  curl -sf -o /dev/null -u "$ADMIN" "$BASE/jmap/session" && break
  sleep 1
done

DOMAIN_ID="$(jmap "$ADMIN" '{"using":["urn:ietf:params:jmap:core","urn:stalwart:jmap"],"methodCalls":[["x:Domain/get",{"ids":null,"properties":["id","name"]},"c"]]}' \
  | json '[x["id"] for x in d["methodResponses"][0][1]["list"] if x["name"]=="example.com"][0]')"
for user in alice bob; do
  jmap "$ADMIN" '{"using":["urn:ietf:params:jmap:core","urn:stalwart:jmap"],"methodCalls":[["x:Account/set",{"create":{"n":{"@type":"User","name":"'$user'","domainId":"'$DOMAIN_ID'","credentials":{"0":{"@type":"Password","secret":"Test-Passw0rd-'$user'"}},"roles":{"@type":"User"},"permissions":{"@type":"Inherit"},"encryptionAtRest":{"@type":"Disabled"}}}},"c"]]}' >/dev/null
done

# an OAuth client, as Caley's OIDC client would be registered when Stalwart is the identity provider
CLIENT="$(curl -sf -u "$ADMIN" -X POST "$BASE/auth/register" -H 'content-type: application/json' \
  -d '{"redirect_uris":["http://127.0.0.1:3000/auth/oidc"],"client_name":"caley-test","grant_types":["authorization_code","refresh_token"],"token_endpoint_auth_method":"client_secret_basic","scope":"openid offline_access urn:ietf:params:oauth:scope:mail"}')"

cat > "$OUT" <<EOF
STALWART_ADMIN=$ADMIN
JMAP_TEST_URL=$BASE
JMAP_TEST_USER=alice@example.com
JMAP_TEST_PASSWORD=Test-Passw0rd-alice
JMAP_TEST_TO=bob@example.com
JMAP_TEST_TO_PASSWORD=Test-Passw0rd-bob
JMAP_TEST_OAUTH_CLIENT_ID=$(echo "$CLIENT" | json 'd["client_id"]')
JMAP_TEST_OAUTH_CLIENT_SECRET=$(echo "$CLIENT" | json 'd["client_secret"]')
JMAP_TEST_OAUTH_REDIRECT=http://127.0.0.1:3000/auth/oidc
JMAP_TEST_REWRITE=https://mail.example.com=$BASE
EOF
echo "provisioned: $OUT"
cat "$OUT"
