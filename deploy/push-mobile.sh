#!/bin/sh
# 모바일 PWA + Runner를 원격 호스트(기본 mini)에 배포한다.
#
# 사용:
#   deploy/push-mobile.sh [ssh-target] [branch]
#   deploy/push-mobile.sh mini dev
set -eu

target="${1:-mini}"
branch="${2:-$(git rev-parse --abbrev-ref HEAD)}"
remote_src="${PRAXIS_REMOTE_SRC:-~/praxis-build}"
bundle_file="$(mktemp -u)/praxis-mobile.bundle"
mkdir -p "$(dirname "$bundle_file")"

repo_root="$(git rev-parse --show-toplevel)"
cd "$repo_root"

if ! git diff --quiet || ! git diff --cached --quiet; then
  echo "error: 커밋되지 않은 변경이 있습니다. 커밋 후 다시 실행하세요." >&2
  git status --short >&2
  exit 1
fi

echo "==> 모바일 번들 빌드"
npm run build:mobile

echo "==> git bundle 생성 ($branch)"
git bundle create "$bundle_file" "$branch"

echo "==> 전송 -> $target"
scp -q "$bundle_file" "$target:~/praxis-mobile.bundle"
scp -q -r dist-mobile "$target:$remote_src/"
rm -f "$bundle_file"

echo "==> 원격 동기화 · 빌드 · 서비스 교체"
ssh "$target" "set -eu
  cd $remote_src
  git fetch ~/praxis-mobile.bundle '$branch:refs/remotes/bundle/mobile'
  git checkout -f -B mobile refs/remotes/bundle/mobile
  . ~/.cargo/env
  cd src-tauri
  cargo build --release --bin praxis-runner
  cp ~/.local/bin/praxis-runner ~/.local/bin/praxis-runner.bak-\$(date +%Y%m%d) || true
  systemctl --user stop praxis-runner
  cp target/release/praxis-runner ~/.local/bin/praxis-runner
  systemctl --user start praxis-runner
"

echo "==> 검증"
ssh "$target" "set -eu
  sleep 2
  systemctl --user is-active praxis-runner
  printf '/m/       : '; curl -sS -o /dev/null -w '%{http_code}\n' http://127.0.0.1:47831/m/
  printf 'v1(무인증): '; curl -sS -o /dev/null -w '%{http_code}\n' http://127.0.0.1:47831/v1/health
"
