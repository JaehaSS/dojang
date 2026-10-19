#!/bin/bash
# RHEL 8.4 x86_64: Docker 20.10.24 / Compose 2.17.2
# Prerequisite: approved RHEL 8 BaseOS + AppStream repositories are available.
set -euo pipefail

if [[ -f /etc/docker/daemon.json ]]; then
  echo 'Existing /etc/docker/daemon.json: preserve it and configure storage manually.' >&2
  exit 1
fi

# Dependencies. dnf resolves their additional RPM dependencies automatically.
dnf --exclude='kernel*,redhat-release*' install -y \
  ca-certificates container-selinux libseccomp fuse-overlayfs slirp4netns

cat > /etc/yum.repos.d/docker-ce.repo <<'REPO'
[docker-ce-stable]
name=Docker CE Stable - EL8
baseurl=https://download.docker.com/linux/centos/8/x86_64/stable
enabled=1
gpgcheck=1
gpgkey=https://download.docker.com/linux/centos/gpg
sslverify=1
REPO

dnf --exclude='kernel*,redhat-release*' install -y \
  docker-ce-20.10.24-3.el8 \
  docker-ce-cli-20.10.24-3.el8 \
  docker-ce-rootless-extras-20.10.24-3.el8 \
  containerd.io-1.6.20-3.1.el8 \
  docker-compose-plugin-2.17.2-1.el8

mkdir -p /etc/docker
printf '{"storage-driver":"overlay2"}\n' > /etc/docker/daemon.json
systemctl enable --now docker
