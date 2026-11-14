#!/usr/bin/env python3
"""Offline source-contract checks and optional isolated native CI negative control.

Requires Python 3.10+ and existing PyYAML; never installs tools. This is not a
GitHub Actions emulator. See docs/CI_LOCAL_VERIFICATION.md for scope and limits.
"""
from __future__ import annotations

import argparse
import copy
import hashlib
import json
import os
from pathlib import Path
import re
import shutil
import subprocess
import sys
import tempfile
from datetime import datetime, timezone

try:
    import yaml
except ImportError:
    sys.exit("BLOCKED: existing PyYAML is required; nothing was installed")

ROOT = Path(__file__).resolve().parents[1]
SLSA = "slsa-framework/slsa-github-generator/.github/workflows/generator_generic_slsa3.yml@v2.1.0"
PINS = {
    "actions/checkout": {"11d5960a326750d5838078e36cf38b85af677262", "93ea575cb5d8a053eaa0ac8fa3b40d7e05a33cc8"},
    "actions/upload-artifact": {"ea165f8d65b6e75b540449e92b4886f43607fa02"},
    "actions/download-artifact": {"d3f86a106a0bac45b974a628896c90dbdf5c8093"},
    "softprops/action-gh-release": {"9d7c94cfd0a1f3ed45544c887983e9fa900f0564"},
    "codecov/codecov-action": {"84508663e988701840491b86de86b666e8a86bed"},
    "ossf/scorecard-action": {"0864cf19026789058feabb7e87baa5f140aac736"},
    "github/codeql-action/upload-sarif": {"17573ee1cc1b9d061760f3a006fc4aac4f944fd5"},
}
IMAGE = "mcr.microsoft.com/devcontainers/rust:1-bookworm@sha256:86d9228ec0744191b45ec91e2bb9a0dd7d7bfd4c4f01ef47abb78032ddddf1c7"
GATES = [
    "cargo +1.96.1 fmt --all -- --check",
    "cargo +1.96.1 clippy --workspace --all-targets --locked -- -D warnings",
    "cargo +1.96.1 test --workspace --locked",
    "bash scripts/package-release-test.sh",
]
TAG_IF = "github.event_name == 'push' && startsWith(github.ref, 'refs/tags/v')"
INPUTS = ["Cargo.toml", "Cargo.lock", "rust-toolchain.toml", "src", "tests", "scripts", "docs", "README.md", "LICENSE", ".github", ".devcontainer", "bourso-launchd.sh"]


class StrictLoader(yaml.SafeLoader):
    """Preserve Actions' 'on' key (YAML 1.2), reject ambiguous duplicate keys."""


StrictLoader.yaml_implicit_resolvers = copy.deepcopy(yaml.SafeLoader.yaml_implicit_resolvers)
for key, resolvers in StrictLoader.yaml_implicit_resolvers.items():
    StrictLoader.yaml_implicit_resolvers[key] = [
        (tag, pattern) for tag, pattern in resolvers if tag != "tag:yaml.org,2002:bool"
    ]
StrictLoader.add_implicit_resolver("tag:yaml.org,2002:bool", re.compile(r"^(?:true|false)$", re.I), list("tTfF"))


def unique_mapping(loader, node, deep=False):
    result = {}
    for key_node, value_node in node.value:
        key = loader.construct_object(key_node, deep=deep)
        if key in result:
            raise ValueError(f"duplicate YAML key: {key}")
        result[key] = loader.construct_object(value_node, deep=deep)
    return result


StrictLoader.add_constructor(yaml.resolver.BaseResolver.DEFAULT_MAPPING_TAG, unique_mapping)


def require(condition, message):
    if not condition:
        raise ValueError(message)


def walk(value):
    if isinstance(value, dict):
        yield value
        for child in value.values():
            yield from walk(child)
    elif isinstance(value, list):
        for child in value:
            yield from walk(child)


def inspect(workflows, dev):
    """Fail closed for this reviewed workflow shape; changes need explicit review."""
    require(set(workflows) == {"cicd.yml", "coverage.yml", "scorecard.yml"}, "workflow inventory changed")
    inventory = []
    for filename, workflow in workflows.items():
        require(workflow["permissions"] == ("read-all" if filename == "scorecard.yml" else {"contents": "read"}), f"{filename}: default permissions")
        for node in walk(workflow):
            require("continue-on-error" not in node, f"{filename}: hidden failure tolerance")
            require("container" not in node and "services" not in node, f"{filename}: unreviewed container")
            if "uses" in node:
                ref = node["uses"]
                if ref == SLSA:
                    require(filename == "cicd.yml" and node is workflow["jobs"]["provenance"], "SLSA exception scope")
                elif ref.startswith("./"):
                    require(ref == "./.github/workflows/coverage.yml" and node is workflow["jobs"]["coverage"], "unreviewed local workflow/action")
                else:
                    name, sep, sha = ref.rpartition("@")
                    require(sep and re.fullmatch(r"[0-9a-f]{40}", sha) and sha in PINS.get(name, set()), f"{filename}: unreviewed/nonimmutable action {ref}")
                inventory.append({"workflow": filename, "uses": ref})
                if ref.startswith("actions/checkout@"):
                    require(node.get("with", {}).get("persist-credentials") is False, "checkout persists credentials")
                    if filename != "scorecard.yml":
                        require(node["with"].get("ref") == "${{ github.sha }}", "checkout must use exact event SHA")
            require(node.get("secrets") is None, "reusable workflow must not inherit secrets")
        for job_id, job in workflow["jobs"].items():
            expected = {"contents": "read"}
            if (filename, job_id) == ("cicd.yml", "publish"):
                expected = {"contents": "write"}
            elif (filename, job_id) == ("cicd.yml", "provenance"):
                expected = {"actions": "read", "id-token": "write", "contents": "write"}
            elif filename == "scorecard.yml":
                expected = {"security-events": "write", "id-token": "write"}
            require(job.get("permissions", workflow["permissions"]) == expected, f"{filename}/{job_id}: permissions")
            # Only the optional Codecov push step may mention a secret. Check
            # the rest of the entire document separately below, including env.
            for step in job.get("steps", []):
                if step.get("uses", "").startswith("codecov/codecov-action@"):
                    require(filename == "coverage.yml" and step.get("if") == "github.event_name == 'push'", "Codecov secret must be push-only")
                    require(step.get("with", {}).get("token") == "${{ secrets.CODECOV_TOKEN }}", "unexpected Codecov token expression")
        scrubbed = copy.deepcopy(workflow)
        for node in walk(scrubbed):
            if node.get("uses", "").startswith("codecov/codecov-action@"):
                node.get("with", {}).pop("token", None)
        require(not re.search(r"\bsecrets\s*(?:\.|\[)", json.dumps(scrubbed)), f"{filename}: unreviewed secret reference")

    ci = workflows["cicd.yml"]
    require(ci["on"].get("pull_request", "missing") is None, "PR gate must be unfiltered")
    require(ci["on"]["push"] == {"branches": ["main"], "tags": ["v*.*.*"]}, "push/tag trigger changed")
    require(ci["env"]["RUSTUP_TOOLCHAIN"] == "1.96.1", "compiler pin")
    jobs = ci["jobs"]
    edges = {"gates": [], "coverage": [], "build": ["gates", "coverage"], "subjects": ["build"], "publish": ["gates", "coverage", "subjects"], "provenance": ["subjects", "publish"]}
    require(set(jobs) == set(edges), "release job inventory changed")
    for name, needs in edges.items():
        require(jobs[name].get("needs", []) == needs, f"{name}: release DAG changed")
        require(jobs[name].get("if") == (TAG_IF if name in {"build", "publish"} else None), f"{name}: bypass/condition changed")
    gate_steps = jobs["gates"]["steps"]
    install = "rustup set auto-self-update disable\nrustup toolchain install 1.96.1 --profile minimal --component rustfmt --component clippy\n"
    require([s["run"] for s in gate_steps if "run" in s] == [install, *GATES], "mandatory gate commands/order changed")
    require(len(gate_steps) == 6 and all("if" not in s for s in gate_steps), "conditional/extra gate step")
    require(all("if" not in s for s in jobs["build"]["steps"]), "conditional build step")
    require(jobs["gates"]["strategy"]["matrix"] == {"os": ["ubuntu-24.04", "macos-15-intel", "windows-2022"]}, "platform gates changed")
    for name in ["gates", "build"]:
        require(jobs[name]["steps"][0]["uses"].startswith("actions/checkout@"), f"{name}: missing checkout")
    require(jobs["coverage"]["uses"] == "./.github/workflows/coverage.yml", "coverage must use same-revision reusable workflow")
    coverage = workflows["coverage.yml"]
    require(set(coverage["jobs"]) == {"test"}, "coverage jobs changed")
    require(set(coverage["on"]) == {"push", "pull_request", "workflow_call"}, "coverage triggers changed")
    covjob = coverage["jobs"]["test"]
    require("if" not in covjob and "needs" not in covjob, "coverage bypass")
    covsteps = covjob["steps"]
    require(covsteps[0]["with"]["ref"] == "${{ github.sha }}", "coverage exact SHA")
    require(any(s.get("run") == "bash scripts/coverage.sh" and "if" not in s for s in covsteps), "coverage script not mandatory")
    require(not any(s.get("uses", "").startswith("actions/checkout@") for s in jobs["publish"]["steps"]), "publisher must not check out source")
    require([s["run"] for s in jobs["publish"]["steps"] if "run" in s] == ["sha256sum --check SHA256SUMS"], "publisher may only verify assets, not execute source")
    require(jobs["provenance"]["with"]["base64-subjects"] == "${{ needs.subjects.outputs.hashes }}", "provenance subject binding")
    require(dev["image"] == IMAGE, "devcontainer image digest changed")
    require(dev["runArgs"] == ["--name", "bourso-api-dev"] and dev.get("privileged", False) is False, "devcontainer privilege/seccomp override")
    require(not any(k in dev for k in ["capAdd", "securityOpt", "features", "mounts", "containerEnv", "remoteEnv"]), "unreviewed devcontainer capability/credential surface")
    return {"actions": inventory, "image": dev["image"], "release_dag": edges}


def mutations(workflows, dev):
    """Deliberately unsafe parsed configs must fail, never modifying YAML on disk."""
    cases = {
        "lint_tolerated": lambda w, d: w["cicd.yml"]["jobs"]["gates"]["steps"][3].update({"continue-on-error": True}),
        "lint_masked": lambda w, d: w["cicd.yml"]["jobs"]["gates"]["steps"][3].update({"run": GATES[1] + " || true"}),
        "gate_skipped": lambda w, d: w["cicd.yml"]["jobs"]["gates"].update({"if": "false"}),
        "publish_bypass": lambda w, d: w["cicd.yml"]["jobs"]["publish"].update({"if": "always()"}),
        "missing_gate_edge": lambda w, d: w["cicd.yml"]["jobs"]["build"].update({"needs": []}),
        "wrong_sha": lambda w, d: w["cicd.yml"]["jobs"]["build"]["steps"][0]["with"].update({"ref": "main"}),
        "floating_action": lambda w, d: w["coverage.yml"]["jobs"]["test"]["steps"][0].update({"uses": "actions/checkout@v4"}),
        "write_build": lambda w, d: w["cicd.yml"]["jobs"]["build"].update({"permissions": {"contents": "write"}}),
        "persisted_credentials": lambda w, d: w["coverage.yml"]["jobs"]["test"]["steps"][0]["with"].update({"persist-credentials": True}),
        "bank_secret": lambda w, d: w["cicd.yml"].update({"env": {"PASSWORD": "${{ secrets.BANK_PASSWORD }}"}}),
        "inherited_secrets": lambda w, d: w["cicd.yml"]["jobs"]["coverage"].update({"secrets": "inherit"}),
        "floating_image": lambda w, d: d.update({"image": "rust:latest"}),
        "unconfined_container": lambda w, d: d["runArgs"].append("--security-opt=seccomp=unconfined"),
    }
    results = []
    for name, mutate in cases.items():
        w, d = copy.deepcopy(workflows), copy.deepcopy(dev)
        mutate(w, d)
        try:
            inspect(w, d)
        except ValueError as error:
            results.append({"name": name, "status": "PASS", "rejected": str(error)})
        else:
            raise ValueError(f"negative source control was not rejected: {name}")
    return results


def hashes(root):
    result = {}
    for name in INPUTS:
        path = root / name
        require(path.exists(), f"missing input: {name}")
        for file in sorted(path.rglob("*")) if path.is_dir() else [path]:
            require(not file.is_symlink(), f"symlink input not supported: {file}")
            if file.is_file():
                result[str(file.relative_to(root))] = hashlib.sha256(file.read_bytes()).hexdigest()
    return result


def native(output, toolchain_bin, timeout):
    before = hashes(ROOT)
    source = output / "source"
    source.mkdir()
    for name in INPUTS:
        src, dest = ROOT / name, source / name
        if src.is_dir():
            shutil.copytree(src, dest)
        else:
            shutil.copy2(src, dest)
    require(hashes(source) == before, "inputs changed while copying; retry after writers stop")
    (output / "source-hashes.json").write_text(json.dumps(before, indent=2) + "\n")
    home = output / "home"
    home.mkdir()
    # Do not pass bank, GitHub, registry, proxy, or unrelated credentials to gates.
    env = {k: os.environ[k] for k in ["PATH", "SYSTEMROOT", "WINDIR", "PATHEXT"] if k in os.environ}
    env.update({"HOME": str(home), "USERPROFILE": str(home), "XDG_CONFIG_HOME": str(home / ".config"), "CARGO_HOME": os.environ.get("CARGO_HOME", str(Path.home() / ".cargo")), "RUSTUP_HOME": os.environ.get("RUSTUP_HOME", str(Path.home() / ".rustup")), "RUSTUP_TOOLCHAIN": "1.96.1", "RUSTUP_AUTO_INSTALL": "0", "CARGO_NET_OFFLINE": "true", "CARGO_TARGET_DIR": str(output / "target"), "CARGO_TERM_COLOR": "never", "TMPDIR": str(output), "TEMP": str(output), "TMP": str(output), "LC_ALL": "C"})
    if toolchain_bin:
        env["PATH"] = str(toolchain_bin.resolve()) + os.pathsep + env["PATH"]
    records = []

    def run(name, command):
        started = datetime.now(timezone.utc).isoformat()
        with (output / f"{name}.log").open("w") as log:
            process = subprocess.run(command, cwd=source, env=env, stdout=log, stderr=subprocess.STDOUT, timeout=timeout, check=False)
        records.append({"name": name, "command": command, "cwd": str(source), "returncode": process.returncode, "log": f"{name}.log", "started_at": started, "finished_at": datetime.now(timezone.utc).isoformat()})
        (output / "commands.json").write_text(json.dumps(records, indent=2) + "\n")
        return process.returncode

    try:
        for name, cmd in [("rustc", ["rustc", "--version"]), ("cargo", ["cargo", "--version"]), ("clippy-version", ["cargo", "clippy", "--version"])]:
            require(run(name, cmd) == 0, f"{name}: tool unavailable (see log)")
        require((output / "rustc.log").read_text().startswith("rustc 1.96.1 "), "native gate requires Rust 1.96.1")
        require((output / "cargo.log").read_text().startswith("cargo 1.96.1 "), "native gate requires Cargo 1.96.1")
        for name, cmd in [
            ("baseline-fmt", ["cargo", "fmt", "--all", "--", "--check"]),
            ("baseline-clippy", ["cargo", "clippy", "--workspace", "--all-targets", "--locked", "--offline", "--", "-D", "warnings"]),
            ("baseline-tests", ["cargo", "test", "--workspace", "--locked", "--offline"]),
            ("baseline-packaging", ["bash", "scripts/package-release-test.sh"]),
        ]:
            require(run(name, cmd) == 0, f"{name} failed: negative control BLOCKED, not a discriminating failure")
        marker = source / "tests/ci_local_negative_control.rs"
        require(not marker.exists(), "negative-control marker already exists")
        marker.write_text("#[test]\nfn ci_local_negative_control() {\n    let value = 7_u32;\n    let _ = value + 0;\n}\n")
        # A default-warning lint proves -D warnings is effective, unlike a
        # deny-by-default lint which would fail even without the CI policy.
        require(run("warning-clippy", ["cargo", "clippy", "--workspace", "--all-targets", "--locked", "--offline", "--message-format=json"]) == 0, "injected warning must be nonfatal without -D warnings")
        warning_seen = False
        for line in (output / "warning-clippy.log").read_text().splitlines():
            try:
                item = json.loads(line)
            except ValueError:
                continue
            if item.get("reason") == "compiler-message":
                message = item["message"]
                warning_seen |= (message.get("level") == "warning" and (message.get("code") or {}).get("code") == "clippy::identity_op" and any(s["file_name"].endswith("tests/ci_local_negative_control.rs") for s in message["spans"]))
        require(warning_seen, "injected warning not observed")
        code = run("negative-clippy", ["cargo", "clippy", "--workspace", "--all-targets", "--locked", "--offline", "--message-format=json", "--", "-D", "warnings"])
        diagnostics = []
        for line in (output / "negative-clippy.log").read_text().splitlines():
            try:
                item = json.loads(line)
            except ValueError:
                continue
            if item.get("reason") == "compiler-message" and item["message"].get("level") == "error":
                diagnostics.append(item["message"])
        require(code == 101 and diagnostics, "negative Clippy did not produce expected compiler failure")
        require(all((d.get("code") or {}).get("code") == "clippy::identity_op" and any(s["file_name"].endswith("tests/ci_local_negative_control.rs") for s in d["spans"]) for d in diagnostics), "unrelated compile/lint error: negative control is NOT discriminating")
        marker.unlink()
        require(hashes(source) == before, "gate changed copied inputs")
        return {"status": "PASS", "commands": records, "negative_diagnostic": "clippy::identity_op", "source_hashes": "source-hashes.json"}
    finally:
        require(hashes(ROOT) == before, "repository inputs changed during execution; evidence is stale, rerun when stable")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--output", type=Path, help="new evidence directory under /tmp (default: unique temporary directory)")
    parser.add_argument("--negative-control", action="store_true", help="also run clean native gates, then inject Clippy lint in a copied source tree")
    parser.add_argument("--toolchain-bin", type=Path, help="existing Rust 1.96.1 bin directory; no installation")
    parser.add_argument("--timeout", type=int, default=600, help="seconds per native command")
    args = parser.parse_args()
    output = args.output.resolve() if args.output else Path(tempfile.mkdtemp(prefix="bourso-ci-local-", dir="/tmp"))
    require(output != Path("/tmp") and Path("/tmp") in output.parents and ROOT not in output.parents, "output must be a new directory under /tmp, outside repository")
    if args.output:
        output.mkdir(parents=True, exist_ok=False)
    report = {"started_at": datetime.now(timezone.utc).isoformat(), "source_contract": "NOT RUN", "native": {"status": "NOT RUN"}, "remote_ci": "NOT RUN", "provenance": "local source hashes only; coordinator must freeze candidate for independent acceptance"}
    try:
        files = sorted((ROOT / ".github/workflows").glob("*.y*ml"))
        workflows = {f.name: yaml.load(f.read_text(), Loader=StrictLoader) for f in files}
        dev = json.loads((ROOT / ".devcontainer/devcontainer.json").read_text())
        report["inventory"] = inspect(workflows, dev)
        report["source_negative_controls"] = mutations(workflows, dev)
        report["source_contract"] = "PASS"
        report["config_hashes"] = {str(p.relative_to(ROOT)): hashlib.sha256(p.read_bytes()).hexdigest() for p in files + [ROOT / ".devcontainer/devcontainer.json", Path(__file__).resolve()]}
        if args.negative_control:
            report["native"] = {"status": "BLOCKED"}
            report["native"] = native(output, args.toolchain_bin, args.timeout)
        report["status"] = "PASS"
    except (ValueError, KeyError, TypeError, OSError, yaml.YAMLError, subprocess.TimeoutExpired) as error:
        report.update(status="FAIL", error=str(error))
    (output / "report.json").write_text(json.dumps(report, indent=2) + "\n")
    print(f"{report['status']}: {output / 'report.json'}")
    if "error" in report:
        print(report["error"], file=sys.stderr)
    return 0 if report["status"] == "PASS" else 1


if __name__ == "__main__":
    sys.exit(main())
