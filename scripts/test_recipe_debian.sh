#!/usr/bin/env bash
# Automated Hermetic Integration Test for Debian/Ubuntu Deployment Recipe
# Validates dbsrt-agent binary installation, systemd user service lifecycle,
# post-reboot persistence, and clean uninstallation in an isolated systemd container.
set -euo pipefail

IMAGE_NAME="dbsrt-test-debian:latest"
DOCKERFILE_PATH="docker/Dockerfile.test-debian-systemd"
CONTAINER_NAME="dbsrt-recipe-test-$$"

TEST_USER_NAME="testuser"
TEST_USER_ID=1000
TEST_PORT=8765

CONTAINER_STARTUP_WAIT_SECONDS="2"
CONTAINER_RESTART_WAIT_SECONDS="2"
HEALTHCHECK_POLL_INTERVAL_SECONDS="0.5"
HEALTHCHECK_MAX_ATTEMPTS=20

if [[ -t 1 && -z "${NO_COLOR:-}" ]]; then
  COLOR_RESET="\033[0m"
  COLOR_BOLD="\033[1m"
  COLOR_GREEN="\033[0;32m"
  COLOR_RED="\033[0;31m"
else
  COLOR_RESET=""
  COLOR_BOLD=""
  COLOR_GREEN=""
  COLOR_RED=""
fi

cleanup() {
  if docker ps -a --format '{{.Names}}' | grep -q "^${CONTAINER_NAME}$"; then
    echo "Cleaning up ephemeral test container ${CONTAINER_NAME}..."
    docker rm -f "${CONTAINER_NAME}" >/dev/null 2>&1 || true
  fi
}
trap cleanup EXIT INT TERM

echo -e "${COLOR_BOLD}=== [Recipe Test] 1. Ensuring test image ${IMAGE_NAME} exists ===${COLOR_RESET}"
if ! docker image inspect "${IMAGE_NAME}" >/dev/null 2>&1; then
  echo "Building systemd-enabled Debian test container..."
  docker build -t "${IMAGE_NAME}" -f "${DOCKERFILE_PATH}" docker/
else
  echo "Using existing image ${IMAGE_NAME}"
fi

# Ensure release binary exists on host
if [[ ! -f "target/release/dbsrt-server" ]]; then
  echo "Building release binary dbsrt-server on host..."
  cargo build --release --bin dbsrt-server
fi

echo -e "${COLOR_BOLD}=== [Recipe Test] 2. Starting ephemeral systemd container ===${COLOR_RESET}"
docker run -d \
  --name "${CONTAINER_NAME}" \
  --privileged \
  --cgroupns=host \
  -v /sys/fs/cgroup:/sys/fs/cgroup:rw \
  --tmpfs /run \
  --tmpfs /run/lock \
  "${IMAGE_NAME}" >/dev/null

sleep "${CONTAINER_STARTUP_WAIT_SECONDS}"

# Verify systemd PID 1 is active
if ! docker exec "${CONTAINER_NAME}" systemctl is-system-running >/dev/null 2>&1; then
  echo -e "${COLOR_RED}❌ Container systemd failed to initialize.${COLOR_RESET}" >&2
  exit 1
fi
echo -e "${COLOR_GREEN}✓ Container systemd PID 1 initialized successfully.${COLOR_RESET}"

echo -e "${COLOR_BOLD}=== [Recipe Test] 3. Copying binaries & deployment scripts ===${COLOR_RESET}"
docker exec -u "${TEST_USER_NAME}" "${CONTAINER_NAME}" mkdir -p \
  "/home/${TEST_USER_NAME}/pydbsrt/target/release" \
  "/home/${TEST_USER_NAME}/pydbsrt/vlc-plugin" \
  "/home/${TEST_USER_NAME}/pydbsrt/scripts"

docker cp target/release/dbsrt-server "${CONTAINER_NAME}:/home/${TEST_USER_NAME}/pydbsrt/target/release/dbsrt-server"
docker cp vlc-plugin/vlsub_dbsrt.lua "${CONTAINER_NAME}:/home/${TEST_USER_NAME}/pydbsrt/vlc-plugin/vlsub_dbsrt.lua"
docker cp scripts/install_debian.sh "${CONTAINER_NAME}:/home/${TEST_USER_NAME}/pydbsrt/scripts/install_debian.sh"
docker exec -u root "${CONTAINER_NAME}" chown -R "${TEST_USER_NAME}:${TEST_USER_NAME}" "/home/${TEST_USER_NAME}/pydbsrt"

echo -e "${COLOR_BOLD}=== [Recipe Test] 4. Executing install recipe as unprivileged user ===${COLOR_RESET}"
docker exec \
  -u "${TEST_USER_NAME}" \
  -w "/home/${TEST_USER_NAME}/pydbsrt" \
  -e "XDG_RUNTIME_DIR=/run/user/${TEST_USER_ID}" \
  -e "DBUS_SESSION_BUS_ADDRESS=unix:path=/run/user/${TEST_USER_ID}/bus" \
  "${CONTAINER_NAME}" \
  ./scripts/install_debian.sh install

echo -e "${COLOR_BOLD}=== [Recipe Test] 5. Asserting service state & API responsiveness ===${COLOR_RESET}"
# 1. Check unit is active
UNIT_STATE="$(docker exec -u "${TEST_USER_NAME}" -e "XDG_RUNTIME_DIR=/run/user/${TEST_USER_ID}" "${CONTAINER_NAME}" systemctl --user is-active dbsrt-agent.service)"
if [[ "${UNIT_STATE}" != "active" ]]; then
  echo -e "${COLOR_RED}❌ Expected service to be active, got: ${UNIT_STATE}${COLOR_RESET}" >&2
  exit 1
fi
echo -e "${COLOR_GREEN}✓ Service is active (systemctl --user is-active -> ${UNIT_STATE})${COLOR_RESET}"

# 2. Check HTTP health endpoint with retry polling
local_health_ok=0
for ((attempt = 1; attempt <= HEALTHCHECK_MAX_ATTEMPTS; attempt++)); do
  if docker exec -u "${TEST_USER_NAME}" "${CONTAINER_NAME}" curl -s -f "http://127.0.0.1:${TEST_PORT}/v1/health" | grep -q "ok"; then
    local_health_ok=1
    break
  fi
  sleep "${HEALTHCHECK_POLL_INTERVAL_SECONDS}"
done

if [[ "${local_health_ok}" -ne 1 ]]; then
  echo -e "${COLOR_RED}❌ Health endpoint did not respond OK on port ${TEST_PORT}.${COLOR_RESET}" >&2
  exit 1
fi
echo -e "${COLOR_GREEN}✓ Local Agent HTTP health endpoint responsive on port ${TEST_PORT}.${COLOR_RESET}"

echo -e "${COLOR_BOLD}=== [Recipe Test] 6. Simulating host reboot & verifying auto-start ===${COLOR_RESET}"
docker restart "${CONTAINER_NAME}" >/dev/null
sleep "${CONTAINER_RESTART_WAIT_SECONDS}"

REBOOT_STATE="$(docker exec -u "${TEST_USER_NAME}" -e "XDG_RUNTIME_DIR=/run/user/${TEST_USER_ID}" "${CONTAINER_NAME}" systemctl --user is-active dbsrt-agent.service)"
if [[ "${REBOOT_STATE}" != "active" ]]; then
  echo -e "${COLOR_RED}❌ Service did not automatically resume after reboot! State: ${REBOOT_STATE}${COLOR_RESET}" >&2
  exit 1
fi
echo -e "${COLOR_GREEN}✓ Service survived simulated host reboot and resumed automatically.${COLOR_RESET}"

echo -e "${COLOR_BOLD}=== [Recipe Test] 7. Testing clean uninstallation ===${COLOR_RESET}"
docker exec \
  -u "${TEST_USER_NAME}" \
  -w "/home/${TEST_USER_NAME}/pydbsrt" \
  -e "XDG_RUNTIME_DIR=/run/user/${TEST_USER_ID}" \
  -e "DBUS_SESSION_BUS_ADDRESS=unix:path=/run/user/${TEST_USER_ID}/bus" \
  "${CONTAINER_NAME}" \
  ./scripts/install_debian.sh uninstall

# Assert unit is stopped/removed
if docker exec -u "${TEST_USER_NAME}" -e "XDG_RUNTIME_DIR=/run/user/${TEST_USER_ID}" "${CONTAINER_NAME}" systemctl --user is-active dbsrt-agent.service >/dev/null 2>&1; then
  echo -e "${COLOR_RED}❌ Service still active after uninstallation!${COLOR_RESET}" >&2
  exit 1
fi

# Assert binary removed
if docker exec "${CONTAINER_NAME}" test -f "/home/${TEST_USER_NAME}/.local/bin/dbsrt-agent"; then
  echo -e "${COLOR_RED}❌ Binary still exists after uninstallation!${COLOR_RESET}" >&2
  exit 1
fi

echo -e "${COLOR_GREEN}✓ Clean uninstallation confirmed (service inactive, binary removed).${COLOR_RESET}"

echo ""
echo -e "${COLOR_BOLD}${COLOR_GREEN}================================================================================${COLOR_RESET}"
echo -e "${COLOR_BOLD}${COLOR_GREEN}  🎉 ALL DEPLOYMENT RECIPE TESTS PASSED (100% Hermetic in Docker systemd)       ${COLOR_RESET}"
echo -e "${COLOR_BOLD}${COLOR_GREEN}================================================================================${COLOR_RESET}"
