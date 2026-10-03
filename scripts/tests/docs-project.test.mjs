/**
 * `docs/lessons.json` 투영(설계 §6.2) — `docs:project`가 실제로 파일 트리를 내는지 픽스처
 * 저장소에서 확인한다. 이 저장소를 상대로는 docs/subjects.yml·docs/memory/ 원본을 건드리게 되어
 * 검증할 수 없다 — `PRAXIS_DOCS_ROOT`가 그 이음매다(`docs-renumber.test.mjs`와 같은 패턴).
 */
import { execFileSync } from 'node:child_process'
import { mkdtempSync, mkdirSync, writeFileSync, readFileSync, rmSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { join, dirname } from 'node:path'
import { fileURLToPath } from 'node:url'
import { describe, it, expect, beforeEach, afterEach } from 'vitest'

const SCRIPT = join(dirname(fileURLToPath(import.meta.url)), '../docs-project.mjs')

let root

function write(rel, body) {
  mkdirSync(join(root, dirname(rel)), { recursive: true })
  writeFileSync(join(root, rel), body, 'utf8')
}

function run() {
  return execFileSync('node', [SCRIPT], {
    cwd: root,
    encoding: 'utf8',
    env: { ...process.env, PRAXIS_DOCS_ROOT: root },
  })
}

function readLessonsJson() {
  return JSON.parse(readFileSync(join(root, 'docs/lessons.json'), 'utf8'))
}

beforeEach(() => {
  root = mkdtempSync(join(tmpdir(), 'docs-project-'))
  write('docs/subjects.yml', 'subjects:\n  a/b: { title: "에이비" }\n')
  // 중복 lesson 필드 — 첫 줄만 고르면 두 번째 교훈이 사라진다(§6.1의 사고를 재현).
  write(
    'docs/memory/0002-two.md',
    [
      '### #2 · 2026-01-02 · 둘째 (Small)',
      '',
      '- **subject**: a/b',
      '- **status**: done',
      '- **lesson**: 첫 번째 교훈',
      '- **lesson**: 두 번째 교훈',
      '- **abandoned**: 버린 길 하나',
      '',
    ].join('\n'),
  )
  write(
    'docs/memory/0001-one.md',
    ['### #1 · 2026-01-01 · 첫째 (Small)', '', '- **subject**: a/b', '- **status**: done', ''].join('\n'),
  )
})

afterEach(() => rmSync(root, { recursive: true, force: true }))

describe('docs:project — docs/lessons.json', () => {
  it('스키마 1, 번호 내림차순, lesson·abandoned를 배열 전체로 낸다', () => {
    run()
    const out = readLessonsJson()
    expect(out.schema).toBe(1)
    expect(out.entries.map((e) => e.number)).toEqual([2, 1])

    const two = out.entries[0]
    expect(two).toEqual({
      number: 2,
      date: '2026-01-02',
      subjects: ['a/b'],
      status: 'done',
      lessons: ['첫 번째 교훈', '두 번째 교훈'],
      abandoned: ['버린 길 하나'],
    })

    const one = out.entries[1]
    expect(one.lessons).toEqual([])
    expect(one.abandoned).toEqual([])
  })

  it('생성 시각 없이 결정론적이다 — 두 번 돌려도 같은 바이트', () => {
    run()
    const first = readFileSync(join(root, 'docs/lessons.json'), 'utf8')
    run()
    const second = readFileSync(join(root, 'docs/lessons.json'), 'utf8')
    expect(second).toBe(first)
  })
})
