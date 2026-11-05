"""Local build dependency inventory; records declared licenses, not legal approval.

Includes resolved Cargo workspace dependencies and installed pnpm dependencies
(including development/build tooling). It does not claim every component ships.
"""
import argparse
import hashlib
import json
import pathlib
import subprocess
import uuid
from datetime import datetime, timezone

ROOT = pathlib.Path(__file__).resolve().parent.parent


def command(*args):
    return subprocess.check_output(args, cwd=ROOT, text=True, encoding="utf-8").strip()


def digest(path):
    with path.open("rb") as source:
        return hashlib.file_digest(source, "sha256").hexdigest()


def write(path, value):
    path.write_text(json.dumps(value, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--output", type=pathlib.Path, required=True)
    parser.add_argument("--target-dir", type=pathlib.Path, required=True)
    parser.add_argument("--installer-name", help="Restrict artifacts to this installer and its three native build outputs")
    args = parser.parse_args()
    output = args.output.resolve()
    output.mkdir(parents=True, exist_ok=True)
    cargo = json.loads(command("cargo", "metadata", "--locked", "--all-features", "--format-version", "1", "--filter-platform", "x86_64-pc-windows-msvc"))
    node = json.loads(command("pnpm.cmd", "licenses", "list", "--json"))
    components = []
    licenses = []
    for package in cargo["packages"]:
        name, version = package["name"], package["version"]
        license_name = package.get("license") or "UNDECLARED"
        ref = f"pkg:cargo/{name}@{version}"
        components.append({"type": "library", "bom-ref": ref, "name": name,
                           "version": version, "purl": ref,
                           "licenses": [{"license": {"name": license_name}}]})
        licenses.append({"ecosystem": "cargo", "name": name, "version": version,
                         "declared_license": license_name,
                         "license_file_declared": bool(package.get("license_file")),
                         "workspace_member": package["id"] in cargo["workspace_members"]})
    for group in node.values():
        for package in group:
            for version in package["versions"]:
                name = package["name"]
                # Package URL namespace encodes the leading @ of scoped npm names.
                ref = f"pkg:npm/{name.replace('@', '%40')}@{version}"
                license_name = package.get("license") or "UNDECLARED"
                components.append({"type": "library", "bom-ref": ref, "name": name,
                                   "version": version, "purl": ref,
                                   "licenses": [{"license": {"name": license_name}}]})
                licenses.append({"ecosystem": "npm", "name": name, "version": version,
                                 "declared_license": license_name})
    unique = {item["bom-ref"]: item for item in components}
    timestamp = datetime.now(timezone.utc).isoformat().replace("+00:00", "Z")
    write(output / "dependencies.cdx.json", {
        "bomFormat": "CycloneDX", "specVersion": "1.6", "version": 1,
        "serialNumber": f"urn:uuid:{uuid.uuid4()}",
        "metadata": {"timestamp": timestamp, "properties": [
            {"name": "pkms:scope", "value": "Resolved Cargo workspace and installed pnpm dependencies, including build/dev tools; not an exact shipped-binary composition."},
            {"name": "pkms:license-status", "value": "Declared metadata only; not legal review or complete notice text."}]},
        "components": sorted(unique.values(), key=lambda item: item["bom-ref"]),
    })
    write(output / "third-party-licenses.json", licenses)
    files = command("git", "ls-files", "--cached", "--others", "--exclude-standard", "-z").split("\0")
    source_files = [{"path": name, "sha256": digest(ROOT / name)} for name in sorted(set(files))
                    if name and (ROOT / name).is_file()]
    write(output / "source-files.json", source_files)
    release = args.target_dir.resolve() / "release"
    artifacts = []
    for path in sorted(release.rglob("*.exe")):
        if args.installer_name and not (
            path == release / "bundle" / "nsis" / args.installer_name
            or (path.parent == release and path.name in {"spanseek.exe", "pkms-worker.exe", "pkms-inference.exe"})
        ):
            continue
        if path.parent == release or "bundle" in path.relative_to(release).parts:
            artifacts.append({"path": path.relative_to(release).as_posix(),
                              "bytes": path.stat().st_size, "sha256": digest(path)})
    write(output / "build-manifest.json", {
        "generated_at": timestamp,
        "git_revision": command("git", "rev-parse", "HEAD"),
        "working_tree_status": command("git", "status", "--porcelain"),
        "source_inventory_sha256": digest(output / "source-files.json"),
        "lockfiles": {name: digest(ROOT / name) for name in ["Cargo.lock", "pnpm-lock.yaml"]},
        "toolchain": {name: command(name, "--version") for name in ["rustc", "cargo", "node", "pnpm.cmd"]},
        "artifacts": artifacts,
        "pdfium": {
            "revision": (ROOT / ".local-archive/dependencies/pdfium-7881/VERSION").read_text(encoding="utf-8").strip(),
            "dll_sha256": digest(ROOT / ".local-archive/dependencies/pdfium-7881/bin/pdfium.dll"),
            "license_resources": "Bundled PDFium LICENSE and licenses/ directory; separate from package-metadata inventory.",
        },
        "signing": "Internal test artifacts; signing status must be verified separately before distribution.",
        "limitations": "Inventory generated from current working tree. Do not associate it with an earlier build if sources changed; regenerate after the final build. Model weights are separately installed and not represented as bundled components.",
    })
    print(json.dumps({"components": len(unique), "artifacts": len(artifacts),
                      "output": str(output)}, ensure_ascii=False))


if __name__ == "__main__":
    main()
