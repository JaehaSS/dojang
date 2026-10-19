#!/usr/bin/env python3
"""Run the real LspPool against a disposable Java/Gradle project snapshot.

The source project is read-only. Servers, Gradle caches and language-server data
belong to this invocation and are cleaned up even when the test fails.
"""
import argparse
import hashlib
import os
from pathlib import Path
import shutil
import signal
import subprocess
import sys
import tempfile
import threading


def digest(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def snapshot(source, target):
    excluded = {".git", ".gradle", ".praxis", ".dojang", ".loop", ".harness", "build", "out", "node_modules", ".idea"}
    count = 0
    for directory, dirs, files in os.walk(source, followlinks=False):
        dirs[:] = [name for name in dirs if name not in excluded and not Path(directory, name).is_symlink()]
        for name in files:
            file = Path(directory, name)
            if file.is_symlink():
                continue
            if not (file.suffix in {".java", ".gradle"} or name.endswith(".gradle.kts")
                    or name in {"gradle.properties", "gradlew", "gradlew.bat", ".classpath", ".project",
                                "gradle-wrapper.jar", "gradle-wrapper.properties"}
                    or file.parent.name == ".settings" and file.suffix == ".prefs"):
                continue
            destination = target / file.relative_to(source)
            destination.parent.mkdir(parents=True, exist_ok=True)
            shutil.copy2(file, destination)
            count += 1
    return count


def owned_java_processes(private_root):
    output = subprocess.run(["ps", "-ww", "-axo", "pid=,rss=,command="], capture_output=True, text=True, check=True).stdout
    result = []
    for line in output.splitlines():
        parts = line.strip().split(None, 2)
        if len(parts) == 3 and private_root in parts[2] and "java" in parts[2]:
            result.append((int(parts[0]), int(parts[1]), "org.eclipse.equinox.launcher" in parts[2]))
    return result


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("source", type=Path, help="aggregate Java/Gradle repository root")
    parser.add_argument("--path", required=True, help="Java source file relative to the aggregate root")
    parser.add_argument("--workspace", default="mirae", help="editor root relative to the copied aggregate")
    parser.add_argument("--java-home", type=Path, help="test with this project JAVA_HOME while jdtls uses its own runtime")
    args = parser.parse_args()
    source = args.source.resolve(strict=True)
    file = source / args.path
    before = digest(file)
    repository = Path(__file__).resolve().parents[2]
    evidence = repository / ".dojang/verification/java-backend-smoke.log"
    evidence.parent.mkdir(parents=True, exist_ok=True)
    result = 1
    with tempfile.TemporaryDirectory(prefix="dojang-java-backend-") as temporary:
        private = Path(temporary).resolve()
        fixture = private / "project"
        fixture.mkdir()
        count = snapshot(source, fixture)
        assert digest(fixture / args.path) == before
        gradle_home = private / "gradle-home"
        gradle_home.mkdir()
        original_gradle = Path(os.environ.get("GRADLE_USER_HOME", str(Path.home() / ".gradle")))
        for name in ("caches", "wrapper"):
            if (original_gradle / name).is_dir():
                if sys.platform == "darwin":
                    subprocess.run(["cp", "-cR", str(original_gradle / name), str(gradle_home / name)], check=True)
                else:
                    shutil.copytree(original_gradle / name, gradle_home / name)
        editor_root = fixture / args.workspace
        editor_path = (fixture / args.path).relative_to(editor_root).as_posix()
        environment = dict(os.environ, GRADLE_USER_HOME=str(gradle_home), XDG_CACHE_HOME=str(private / "cache"),
                           DOJANG_LSP_JAVA_ROOT=str(editor_root), DOJANG_LSP_JAVA_PATH=editor_path)
        if args.java_home is not None:
            environment["JAVA_HOME"] = str(args.java_home.resolve(strict=True))
        done = threading.Event()
        peak = [0]

        def sample():
            while not done.wait(1):
                for _, rss, is_jdtls in owned_java_processes(str(private)):
                    if is_jdtls:
                        peak[0] = max(peak[0], rss)

        watcher = threading.Thread(target=sample, daemon=True)
        process = None
        try:
            with evidence.open("w") as log:
                watcher.start()
                process = subprocess.Popen(["cargo", "test", "--manifest-path", str(repository / "src-tauri/Cargo.toml"),
                                            "--test", "lspclient_smoke_test", "java_navigation", "-j", "2", "--",
                                            "--ignored", "--nocapture", "--test-threads=1"],
                                           cwd=repository, env=environment, stdout=log, stderr=subprocess.STDOUT,
                                           start_new_session=True)
                result = process.wait(timeout=480)
                if result == 0 and "running 0 tests" in evidence.read_text():
                    result = 2
        finally:
            if process is not None and process.poll() is None:
                os.killpg(process.pid, signal.SIGTERM)
                try:
                    process.wait(timeout=10)
                except subprocess.TimeoutExpired:
                    os.killpg(process.pid, signal.SIGKILL)
                    process.wait()
            done.set()
            if watcher.ident is not None:
                watcher.join(timeout=5)
            if result != 0:
                diagnostic = evidence.with_name("java-backend-import.log")
                with diagnostic.open("w") as log:
                    for metadata in (private / "cache").rglob(".metadata/.log"):
                        log.write(f"\n{metadata.relative_to(private)}\n")
                        log.write(metadata.read_text(errors="replace"))
            try:
                stop = subprocess.run([str(fixture / "gradlew"), "--stop"], cwd=fixture, env=environment,
                                      capture_output=True, text=True, timeout=60)
                with evidence.open("a") as log:
                    log.write(f"\nGradle cleanup exit: {stop.returncode}\n{stop.stdout}{stop.stderr}")
            finally:
                for pid, _, _ in owned_java_processes(str(private)):
                    try:
                        os.kill(pid, signal.SIGKILL)
                    except ProcessLookupError:
                        pass
                assert not owned_java_processes(str(private)), "test-owned Java process survived cleanup"
                assert digest(file) == before, "original Java source changed"
        print(f"Java snapshot: {count} files; source SHA-256 unchanged; exit={result}; sampled jdtls peak={peak[0] / 1024:.1f} MiB")
        print(f"Evidence: {evidence}")
    return result


if __name__ == "__main__":
    raise SystemExit(main())
