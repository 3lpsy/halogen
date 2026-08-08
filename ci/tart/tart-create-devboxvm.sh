#!/usr/bin/env bash
# One-time: create the persistent devboxvm remote-dev clone and join it to the
# tailnet (state persists; later boots auto-connect). Needs TART_TAILSCALE_URL +
# TART_TS_AUTHKEY in .env; seeds admin's ~/.ssh from the TART_SSH_* vars.
# After rebuilding the base image: tart delete devboxvm + remove the stale tailnet node.
set -euo pipefail
script_dir="$(cd "$(dirname "$0")" && pwd)"
# shellcheck source=env.sh
source "${script_dir}/env.sh"

TART_VM_NAME="${TART_VM_NAME:-halogen-builder}"
TART_DEVBOX_VM="${TART_DEVBOX_VM:-devboxvm}"
TART_VM_PASSWORD="${TART_VM_PASSWORD:-admin}"
TART_BOOT_TIMEOUT="${TART_BOOT_TIMEOUT:-300}"
TART_FORGEJO_REPO="${TART_FORGEJO_REPO:-halogen}"

die() { echo "error: $*" >&2; exit 1; }

[ -n "${TART_FORGEJO_OWNER:-}" ] || die "TART_FORGEJO_OWNER not set (.env at repo root)"

# In-cluster Forgejo host — derived from SERVICES_ROOT_DOMAIN (.env at repo root)
# unless TART_FORGEJO_HOST overrides; never hardcoded in the repo.
if [ -z "${TART_FORGEJO_HOST:-}" ]; then
  [ -n "${SERVICES_ROOT_DOMAIN:-}" ] \
    || die "SERVICES_ROOT_DOMAIN not set (.env at repo root) — needed to derive TART_FORGEJO_HOST"
  TART_FORGEJO_HOST="git.${SERVICES_ROOT_DOMAIN}"
fi

command -v tart >/dev/null || die "tart not installed (brew install cirruslabs/cli/tart)"
command -v sshpass >/dev/null \
  || die "sshpass not installed — brew install esolitos/ipa/sshpass (or hudochenkov/sshpass/sshpass)"
[ -n "${TART_TAILSCALE_URL:-}" ] || die "TART_TAILSCALE_URL not set (.env at repo root) — the tailnet login server URL"
[ -n "${TART_TS_AUTHKEY:-}" ] || die "TART_TS_AUTHKEY not set (.env at repo root) — tailnet preauth key for the devbox user"
[ -n "${TART_SSH_PRIV_KEY_PATH:-}" ] || die "TART_SSH_PRIV_KEY_PATH not set (.env at repo root) — host path of the key to inject as the VM's ~/.ssh/id_ed25519"
[ -f "${TART_SSH_PRIV_KEY_PATH}" ] || die "TART_SSH_PRIV_KEY_PATH '${TART_SSH_PRIV_KEY_PATH}' does not exist"
[ -n "${TART_SSH_PUB_KEYS:-}" ] || die "TART_SSH_PUB_KEYS not set (.env at repo root) — CSV of '<type> <key> <comment>' pubkeys for the VM's authorized_keys"
tart get "${TART_DEVBOX_VM}" >/dev/null 2>&1 \
  && die "VM '${TART_DEVBOX_VM}' already exists — 'tart delete ${TART_DEVBOX_VM}' first to recreate"
tart get "${TART_VM_NAME}" >/dev/null 2>&1 || die "VM '${TART_VM_NAME}' not found — run ./tart-build-vm.sh first"

echo "==> cloning '${TART_VM_NAME}' -> '${TART_DEVBOX_VM}' (persistent remote-dev VM)"
tart clone "${TART_VM_NAME}" "${TART_DEVBOX_VM}"

ssh_opts=(
  -o StrictHostKeyChecking=no
  -o UserKnownHostsFile=/dev/null
  -o LogLevel=ERROR
  -o ConnectTimeout=5
)
ip=""
vm_ssh() { sshpass -p "${TART_VM_PASSWORD}" ssh "${ssh_opts[@]}" "admin@${ip}" "$@"; }

# Boot detached — the VM must OUTLIVE this script (mind the 2-running-VM limit).
echo "==> booting '${TART_DEVBOX_VM}' (headless; stays up after this script exits)"
log="${TMPDIR:-/tmp}/halogen-devboxvm.log"
nohup tart run "${TART_DEVBOX_VM}" --no-graphics > "${log}" 2>&1 &
disown
deadline=$(( SECONDS + TART_BOOT_TIMEOUT ))
while [ -z "${ip}" ]; do
  (( SECONDS < deadline )) || die "timed out waiting for the VM's IP (see ${log})"
  ip="$(tart ip "${TART_DEVBOX_VM}" 2>/dev/null || true)"
  [ -n "${ip}" ] || sleep 3
done
echo "==> VM at ${ip}; waiting for SSH"
until vm_ssh true >/dev/null 2>&1; do
  (( SECONDS < deadline )) || die "timed out waiting for SSH"
  sleep 3
done

# Host-side prep: CSV → one pubkey per line; read the private key into memory.
pub_keys="$(tr ',' '\n' <<< "${TART_SSH_PUB_KEYS}" | sed 's/^[[:space:]]*//;s/[[:space:]]*$//' | grep -v '^$')"
priv_key="$(cat "${TART_SSH_PRIV_KEY_PATH}")"

# The auth key + ssh private key expand into a 0600 temp script streamed over
# stdin — they never appear on a VM command line. --reset: these flags ARE the
# full node config.
remote_script="$(mktemp)"
cleanup() { rm -f "${remote_script}"; }
trap cleanup EXIT
cat > "${remote_script}" <<EOF
set -euo pipefail
sudo /opt/homebrew/bin/tailscale up \
  --login-server='${TART_TAILSCALE_URL}' \
  --authkey='${TART_TS_AUTHKEY}' \
  --hostname='${TART_DEVBOX_VM}' \
  --reset
sudo /opt/homebrew/bin/tailscale status | head -5

echo '==> seeding ~/.ssh (identity for forgejo + authorized_keys)'
mkdir -p ~/.ssh && chmod 700 ~/.ssh
cat > ~/.ssh/id_ed25519 <<'PRIVKEY'
${priv_key}
PRIVKEY
chmod 600 ~/.ssh/id_ed25519
cat >> ~/.ssh/authorized_keys <<'PUBKEYS'
${pub_keys}
PUBKEYS
chmod 600 ~/.ssh/authorized_keys
# Pin the forgejo host key now (tailnet is up) so the first clone is non-interactive.
if ssh-keyscan -T 5 '${TART_FORGEJO_HOST}' >> ~/.ssh/known_hosts 2>/dev/null; then
  chmod 600 ~/.ssh/known_hosts
else
  echo "warn: ssh-keyscan ${TART_FORGEJO_HOST} failed — the first git-over-ssh clone will prompt"
fi
EOF
echo "==> joining the tailnet as '${TART_DEVBOX_VM}' via ${TART_TAILSCALE_URL} + seeding ~/.ssh"
vm_ssh "zsh -s" < "${remote_script}"

echo
echo "done — '${TART_DEVBOX_VM}' is on the tailnet and auto-connects on every boot."
echo "  ssh:            ssh admin@${TART_DEVBOX_VM}   (or password '${TART_VM_PASSWORD}')"
echo "  screen sharing: vnc://${TART_DEVBOX_VM}       (user admin, password '${TART_VM_PASSWORD}')"
echo "  git (in VM):    git clone git@${TART_FORGEJO_HOST}:${TART_FORGEJO_OWNER}/${TART_FORGEJO_REPO}.git"
echo "                  (register the injected key's .pub in forgejo → settings → SSH keys)"
echo "  stop / start:   tart stop ${TART_DEVBOX_VM}   /   nohup tart run ${TART_DEVBOX_VM} --no-graphics &"
