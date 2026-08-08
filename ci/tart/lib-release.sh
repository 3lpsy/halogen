# shellcheck shell=bash
# Shared VM lifecycle for the release builders — source, don't execute; call
# release_init <platform> → release_boot → release_run <<EOF → release_scp.
# The ephemeral VM is deleted on exit (TART_KEEP_VM=1 keeps it — token caveat).
# Requires tart + sshpass + TART_FORGEJO_TOKEN.
_lib_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
# shellcheck source=env.sh
source "${_lib_dir}/env.sh"

release_init() {
  _platform="$1"
  TART_TAG="${TART_TAG:-$(halogen_default_tag)}"
  TART_VM_NAME="${TART_VM_NAME:-halogen-builder}"
  TART_VM_PASSWORD="${TART_VM_PASSWORD:-admin}"
  # In-cluster Forgejo host — derived from SERVICES_ROOT_DOMAIN (.env at repo root)
  # unless TART_FORGEJO_HOST overrides; never hardcoded in the repo.
  if [ -z "${TART_FORGEJO_HOST:-}" ]; then
    [ -n "${SERVICES_ROOT_DOMAIN:-}" ] \
      || release_die "SERVICES_ROOT_DOMAIN not set (.env at repo root) — needed to derive TART_FORGEJO_HOST"
    TART_FORGEJO_HOST="git.${SERVICES_ROOT_DOMAIN}"
  fi
  [ -n "${TART_FORGEJO_OWNER:-}" ] || release_die "TART_FORGEJO_OWNER not set (.env at repo root)"
  TART_FORGEJO_REPO="${TART_FORGEJO_REPO:-halogen}"
  TART_BOOT_TIMEOUT="${TART_BOOT_TIMEOUT:-300}"
  TART_OUT_DIR="${TART_OUT_DIR:-${_lib_dir}/artifacts/${TART_TAG}}"
  BUILD_VM="build-${_platform}-${TART_TAG}"

  command -v tart >/dev/null || release_die "tart not installed (brew install cirruslabs/cli/tart)"
  command -v sshpass >/dev/null \
    || release_die "sshpass not installed — brew install esolitos/ipa/sshpass"
  [ -n "${TART_FORGEJO_TOKEN:-}" ] || release_die "TART_FORGEJO_TOKEN not set (.env at repo root)"
  tart get "${TART_VM_NAME}" >/dev/null 2>&1 || release_die "VM '${TART_VM_NAME}' not found — run ./tart-build-vm.sh first"

  _ssh_opts=(
    -o StrictHostKeyChecking=no
    -o UserKnownHostsFile=/dev/null
    -o LogLevel=ERROR
    -o ConnectTimeout=5
  )
  _ip=""
  _run_pid=""
  _remote_script=""
  trap release_cleanup EXIT
}

release_die() { echo "error: $*" >&2; exit 1; }

release_cleanup() {
  rc=$?
  rm -f "${_remote_script}"
  if [ -n "${TART_KEEP_VM:-}" ]; then
    echo "TART_KEEP_VM set — leaving '${BUILD_VM}' around (NOTE: its clone's git config holds the token)"
  elif [ "${rc}" != "0" ]; then
    # Failed runs keep their VM (stopped) automatically — the next run
    # reuses its warm state instead of paying a fresh clone+build.
    tart stop "${BUILD_VM}" >/dev/null 2>&1 || true
    if [ -n "${_run_pid}" ]; then wait "${_run_pid}" 2>/dev/null || true; fi
    echo "kept failed '${BUILD_VM}' (stopped) — the next run reuses it (FRESH_VM=1 for a clean clone; token caveat as TART_KEEP_VM)"
  else
    # Green runs stay ephemeral: delete, so the next run starts fresh from
    # the trusted image.
    tart stop "${BUILD_VM}" >/dev/null 2>&1 || true
    if [ -n "${_run_pid}" ]; then wait "${_run_pid}" 2>/dev/null || true; fi
    tart delete "${BUILD_VM}" >/dev/null 2>&1 || true
  fi
  exit "${rc}"
}

release_vm_ssh() { sshpass -p "${TART_VM_PASSWORD}" ssh "${_ssh_opts[@]}" "admin@${_ip}" "$@"; }

release_boot() {
  if [ -z "${FRESH_VM:-}" ] && tart get "${BUILD_VM}" >/dev/null 2>&1; then
    # Reuse a failed run's leftover VM (warm caches) — but it forfeits the
    # fresh-from-trusted-image guarantee: pass FRESH_VM=1 for upload runs.
    echo "==> ${TART_TAG} [${_platform}]: REUSING leftover '${BUILD_VM}' (FRESH_VM=1 forces a clean clone)"
  else
    echo "==> ${TART_TAG} [${_platform}]: cloning '${TART_VM_NAME}' -> '${BUILD_VM}' (APFS CoW, cheap)"
    tart stop "${BUILD_VM}" >/dev/null 2>&1 || true
    tart delete "${BUILD_VM}" >/dev/null 2>&1 || true
    tart clone "${TART_VM_NAME}" "${BUILD_VM}"
  fi

  # Resource bump for the build clone (the image's own sizing is often the
  # conservative base default). Override via TART_VM_CPU / TART_VM_MEMORY_GB in .env or
  # the shell; mind concurrent VMs — devbox + build must fit the host
  # (memory especially: their sum should stay ~4 GB under physical RAM).
  TART_VM_CPU="${TART_VM_CPU:-8}"
  TART_VM_MEMORY_GB="${TART_VM_MEMORY_GB:-12}"
  echo "==> sizing '${BUILD_VM}': ${TART_VM_CPU} cpus, ${TART_VM_MEMORY_GB} GB"
  tart set "${BUILD_VM}" --cpu "${TART_VM_CPU}" --memory "$(( TART_VM_MEMORY_GB * 1024 ))"

  echo "==> booting (headless)"
  tart run "${BUILD_VM}" --no-graphics &
  _run_pid=$!

  deadline=$(( SECONDS + TART_BOOT_TIMEOUT ))
  while [ -z "${_ip}" ]; do
    (( SECONDS < deadline )) || release_die "timed out (${TART_BOOT_TIMEOUT}s) waiting for the VM's IP"
    kill -0 "${_run_pid}" 2>/dev/null || release_die "'tart run' exited early — run without --no-graphics to see why"
    _ip="$(tart ip "${BUILD_VM}" 2>/dev/null || true)"
    [ -n "${_ip}" ] || sleep 3
  done
  echo "==> VM up at ${_ip}; waiting for SSH"
  until release_vm_ssh true >/dev/null 2>&1; do
    (( SECONDS < deadline )) || release_die "timed out (${TART_BOOT_TIMEOUT}s) waiting for SSH"
    sleep 3
  done
}

# Reads the remote script from stdin. Local vars expand at the CALLER (heredoc
# without quoted delimiter) — the script holds the token, so it stays 0600
# (mktemp) and is removed in cleanup; never echoed.
release_run() {
  _remote_script="$(mktemp)"
  cat > "${_remote_script}"
  release_vm_ssh "zsh -s" < "${_remote_script}"
}

release_scp() {
  sshpass -p "${TART_VM_PASSWORD}" scp -r -q "${_ssh_opts[@]}" \
    "admin@${_ip}:$1" "$2"
}
