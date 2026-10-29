#!/usr/bin/env python3
"""Real native views and stores, local Ollama, isolated data and workspace only."""
import argparse
import json
import os
import pathlib
import shutil
import subprocess
import tempfile
import time

parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument("--model", default="qwen3.5:9b-q4_K_M")
parser.add_argument("--effort", choices=["off", "nominal", "high"], default="nominal")
parser.add_argument("--long", action="store_true", help="Inspect an isolated copy of Lorca's Rust code")
parser.add_argument("--sdk", help="Explicit SDK path; CLT defaults to the installed working 26.5 SDK")
parser.add_argument("--skip-build", action="store_true", help="Use the already-built release core")
args = parser.parse_args()
repo = pathlib.Path(__file__).resolve().parents[2]
root = pathlib.Path(tempfile.mkdtemp(prefix="lorca-native-acceptance-")).resolve()
project = root / "project"
project.mkdir()
print("Acceptance artifacts:", root, flush=True)
fallback = pathlib.Path("/Library/Developer/CommandLineTools/SDKs/MacOSX26.5.sdk")
sdk = args.sdk or (str(fallback) if fallback.exists() and not pathlib.Path("/Applications/Xcode.app").exists() else os.environ.get("SDKROOT"))
sdk = sdk or subprocess.check_output(["xcrun", "--show-sdk-path"], text=True).strip()
if not args.skip_build:
    subprocess.run(["bash", "macos/build.sh"], cwd=repo, check=True, env=dict(os.environ, SDKROOT=sdk, LORCA_BUILD_ONLY="1"))
if args.long:
    for name in ["src", "tests", "assets"]:
        shutil.copytree(repo / name, project / name)
    for name in ["Cargo.toml", "Cargo.lock"]:
        shutil.copy2(repo / name, project / name)
else:
    (project / "src").mkdir()
    (project / "Cargo.toml").write_text('[package]\nname="lorca_native_acceptance"\nversion="0.1.0"\nedition="2021"\n')
    (project / "src/lib.rs").write_text('/// Add two integers.\npub fn add(a: i32, b: i32) -> i32 { a - b }\n\n#[cfg(test)]\nmod tests {\n    #[test]\n    fn adds() { assert_eq!(super::add(2, 3), 5); }\n}\n')
(project / "README.md").write_text("# Old prototype\n\nThis repository is an unfinished Python web server.\n")
sources = sorted(p for p in (repo / "macos/Sources/LorcaMac").glob("*.swift") if p.name != "App.swift")
runner = root / "native-acceptance"
subprocess.run(["xcrun", "swiftc", "-parse-as-library", "-sdk", sdk, "-target", "arm64-apple-macos14.0", "-module-cache-path", str(repo / "macos/.build/out/ModuleCache.noindex"), *map(str, sources), str(pathlib.Path(__file__).with_name("AgentAcceptance.swift")), "-o", str(runner)], check=True, cwd=repo)
env = dict(os.environ, LORCA_DATA_DIR=str(root / "data"), LORCA_ACCEPTANCE_ROOT=str(root), LORCA_ACCEPTANCE_MODEL=args.model, LORCA_ACCEPTANCE_EFFORT=args.effort, LORCA_SOURCE_ROOT=str(repo), PATH=str(pathlib.Path.home() / ".cargo/bin") + ":" + os.environ["PATH"])
if args.long:
    env["LORCA_ACCEPTANCE_TASK"] = "long"
with (root / "backend.log").open("w") as backend_log:
    backend = subprocess.Popen([str(repo / "target/release/lorca"), "serve"], env=env, stdout=backend_log, stderr=backend_log)
    try:
        for _ in range(100):
            if (root / "data/runtime/endpoint.json").exists():
                break
            time.sleep(0.1)
        with (root / "native.log").open("w") as log:
            result = subprocess.run([str(runner)], env=env, stdout=log, stderr=log, timeout=1800)
        print((root / "native.log").read_text(), flush=True)
    finally:
        backend.terminate()
        backend.wait(timeout=10)
raise SystemExit(result.returncode)
