#!/usr/bin/env python3
"""Validate a single reviewed amd64 runtime image without loading or extracting it."""
import argparse
import hashlib
import json
import pathlib
import re
import tarfile


def inspect_archive(path, revision):
    if not re.fullmatch(r"[0-9a-f]{40}", revision):
        raise ValueError("REVIEWED_SHA must be a complete Git SHA")
    tag = f"opportunity-app:{revision}-amd64"
    if path.name != f"opportunity-app-{revision}-amd64.tar.gz":
        raise ValueError("archive name does not match the reviewed revision")
    expected = path.with_name(path.name + ".sha256").read_text().split()[0]
    if not re.fullmatch(r"[0-9a-f]{64}", expected):
        raise ValueError("invalid archive checksum")
    with path.open("rb") as f:
        digest = hashlib.sha256()
        for chunk in iter(lambda: f.read(1024 * 1024), b""):
            digest.update(chunk)
        if digest.hexdigest() != expected:
            raise ValueError("archive checksum mismatch")
    with tarfile.open(path, "r:gz") as archive:
        members = {m.name: m for m in archive.getmembers()}
        if len(members) != len(archive.getmembers()):
            raise ValueError("duplicate archive member")

        def read(name):
            member = members[name]
            if not member.isfile() or member.size > 2_000_000:
                raise ValueError("invalid image metadata member")
            return archive.extractfile(member).read()

        def blob(digest):
            if not re.fullmatch(r"sha256:[0-9a-f]{64}", digest):
                raise ValueError("invalid image digest")
            data = read("blobs/sha256/" + digest[7:])
            if hashlib.sha256(data).hexdigest() != digest[7:]:
                raise ValueError("image metadata digest mismatch")
            return json.loads(data)

        manifests = json.loads(read("manifest.json"))
        if len(manifests) != 1 or manifests[0]["RepoTags"] != [tag]:
            raise ValueError("archive must contain only the intended tag")
        manifest = manifests[0]
        config_bytes = read(manifest["Config"])
        config_id = "sha256:" + hashlib.sha256(config_bytes).hexdigest()
        config = json.loads(config_bytes)
        if config.get("architecture") != "amd64" or config.get("os") != "linux":
            raise ValueError("release must be Linux amd64")
        labels = config.get("config", {}).get("Labels") or {}
        if labels.get("org.opencontainers.image.revision") != revision:
            raise ValueError("image revision does not match reviewed SHA")
        if labels.get("org.opencontainers.image.title") != "opportunity-runtime":
            raise ValueError("image is not the runtime release target")
        for layer in manifest["Layers"]:
            if layer not in members or not members[layer].isfile():
                raise ValueError("archive is missing an image layer")
        identities = [config_id]
        # Docker's classic store identifies configurations; its OCI store may use a manifest/index.
        if "index.json" in members:
            index = json.loads(read("index.json"))
            if len(index["manifests"]) != 1:
                raise ValueError("ambiguous OCI image index")
            descriptor = index["manifests"][0]
            annotations = descriptor.get("annotations", {})
            image_name = annotations.get("io.containerd.image.name")
            if image_name is not None and image_name not in (tag, "docker.io/library/" + tag):
                raise ValueError("OCI image name conflicts with release tag")
            ref = annotations.get("org.opencontainers.image.ref.name")
            if ref is not None and ref != tag.split(":", 1)[1]:
                raise ValueError("OCI reference conflicts with release tag")
            digest = descriptor["digest"]
            node = blob(digest)
            identities.append(digest)
            if "manifests" in node:
                if len(node["manifests"]) != 1:
                    raise ValueError("release must contain one platform without attestations")
                descriptor = node["manifests"][0]
                if descriptor.get("platform") != {"architecture": "amd64", "os": "linux"}:
                    raise ValueError("unexpected OCI platform")
                digest = descriptor["digest"]
                node = blob(digest)
                identities.append(digest)
            if node["config"]["digest"] != config_id:
                raise ValueError("OCI manifest selects a different configuration")
            if ["blobs/sha256/" + item["digest"][7:] for item in node["layers"]] != manifest["Layers"]:
                raise ValueError("OCI and Docker layer selections differ")
        elif manifest["Config"] != config_id[7:] + ".json":
            raise ValueError("configuration filename does not match its digest")
    return tag, ",".join(dict.fromkeys(identities)), expected


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("archive", type=pathlib.Path)
    parser.add_argument("--revision", required=True)
    args = parser.parse_args()
    try:
        print(*inspect_archive(args.archive, args.revision))
    except (ValueError, KeyError, OSError, IndexError, TypeError, tarfile.TarError) as error:
        parser.exit(1, f"Invalid release artifact: {error}\n")


if __name__ == "__main__":
    main()
