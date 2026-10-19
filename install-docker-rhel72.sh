#!/bin/bash
# RHEL 7.2 x86_64: Docker 20.10.24 / Compose 2.17.2
# Prerequisite: approved RHEL 7 Base + Extras repositories are available.
# Original 7.2 kernel: use --lab-devicemapper for a disposable VM,
# or provide an existing production thin pool through DOCKER_DM_THINPOOL.
set -euo pipefail

if [[ -f /etc/docker/daemon.json ]]; then
  echo 'Existing /etc/docker/daemon.json: preserve it and configure storage manually.' >&2
  exit 1
fi
if [[ -z ${DOCKER_DM_THINPOOL:-} && ${1:-} != --lab-devicemapper ]]; then
  echo 'Specify DOCKER_DM_THINPOOL, or use --lab-devicemapper for VM testing.' >&2
  exit 1
fi

# Old RHEL 7.2 RHSM needs TLS SNI. This temporary launcher retains certificate
# verification and does not modify installed Python/RHSM source files.
tmp_dir=$(mktemp -d)
trap 'rm -rf "$tmp_dir"' EXIT
cat > "$tmp_dir/yum-sni.py" <<'PY'
import sys
from M2Crypto import SSL
original_connect = SSL.Connection.connect
def connect_with_sni(self, address):
    if address[0] in ('subscription.rhsm.redhat.com', 'subscription.rhn.redhat.com'):
        self.set_tlsext_host_name(address[0])
    return original_connect(self, address)
SSL.Connection.connect = connect_with_sni
reload(sys)
sys.argv[0] = '/usr/bin/yum'
execfile('/usr/bin/yum', {'__name__': '__main__', '__file__': sys.argv[0]})
PY
yum_install() {
  /usr/bin/python "$tmp_dir/yum-sni.py" \
    --exclude='kernel*,redhat-release*' "$@"
}

# Dependencies and the CA/firewalld updates needed in the RHEL 7.2 VM test.
yum_install update -y ca-certificates firewalld
yum_install install -y libseccomp container-selinux fuse-overlayfs slirp4netns \
  device-mapper-persistent-data lvm2

cat > /etc/yum.repos.d/docker-ce.repo <<'REPO'
[docker-ce-stable]
name=Docker CE Stable - EL7
baseurl=https://download.docker.com/linux/centos/7/x86_64/stable
enabled=1
gpgcheck=1
gpgkey=https://download.docker.com/linux/centos/gpg
sslverify=1
REPO

yum_install install -y \
  docker-ce-20.10.24-3.el7 \
  docker-ce-cli-20.10.24-3.el7 \
  docker-ce-rootless-extras-20.10.24-3.el7 \
  containerd.io-1.6.20-3.1.el7 \
  docker-compose-plugin-2.17.2-1.el7

# RHEL 7.2 original kernel uses devicemapper rather than overlay2.
mkdir -p /etc/docker
if [[ -n ${DOCKER_DM_THINPOOL:-} ]]; then
  /usr/bin/python - "$DOCKER_DM_THINPOOL" <<'PY'
import json, sys
with open('/etc/docker/daemon.json', 'w') as output:
    json.dump({'storage-driver': 'devicemapper', 'storage-opts': [
        'dm.thinpooldev=' + sys.argv[1], 'dm.use_deferred_removal=true']}, output)
PY
else
  # Loop-lvm is only for disposable VM testing.
  printf '{"storage-driver":"devicemapper"}\n' > /etc/docker/daemon.json
fi

if systemctl is-active --quiet firewalld; then
  systemctl restart firewalld
fi
systemctl enable --now docker
