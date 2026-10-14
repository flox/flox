#!/usr/bin/env python3
"""Adversarial fixtures for the image verifier; no Flox or Nix needed."""

import copy
import io
import json
import tarfile
import tempfile
import unittest
from pathlib import Path

from verify_image import InvalidImage, verify_image

CONTEXT = "nix/store/test-context/activations-context"
ENVIRONMENT = "/nix/store/test-environment"
PASSWD = ("root:x:0:0:root user:/var/empty:/bin/sh\n"
          "nobody:x:65534:65534:nobody:/var/empty:/bin/sh\n").encode()
GROUP = b"root:x:0:\nnobody:x:65534:\n"


def regular(data, mode=0o444):
    return (tarfile.REGTYPE, data, mode, 0, 0)


def directory(mode=0o755, uid=0, gid=0):
    return (tarfile.DIRTYPE, b"", mode, uid, gid)


def link(target, kind=tarfile.SYMTYPE):
    return (kind, target, 0o777, 0, 0)


def tar_bytes(entries):
    buffer = io.BytesIO()
    with tarfile.open(fileobj=buffer, mode="w") as archive:
        for name, (kind, data, mode, uid, gid) in entries.items():
            member = tarfile.TarInfo(name)
            member.type, member.mode, member.uid, member.gid = kind, mode, uid, gid
            if kind == tarfile.REGTYPE:
                member.size = len(data)
                archive.addfile(member, io.BytesIO(data))
            else:
                if kind in (tarfile.SYMTYPE, tarfile.LNKTYPE):
                    member.linkname = data
                archive.addfile(member)
    return buffer.getvalue()


class ImageVerifierTests(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory()
        self.addCleanup(self.temporary.cleanup)
        self.path = Path(self.temporary.name) / "image.tar"
        self.context = {
            "mode": "dev",
            "flox_activate_store_path": ENVIRONMENT,
            "attach_ctx": {"flox_env_cuda_detection": "1"},
        }
        self.config = {"config": {
            "Entrypoint": [ENVIRONMENT + "/libexec/flox-activations", "activate",
                           "--activate-data", "/" + CONTEXT],
        }}
        self.files = {
            "etc": directory(),
            "etc/passwd": regular(PASSWD),
            "etc/group": regular(GROUP),
            "tmp": directory(0o1777),
            "run": directory(0o1770),
            "run/flox": directory(0o1770),
            CONTEXT: regular(json.dumps(self.context).encode()),
        }

    def write_image(self, *upper_layers, reverse_tar_order=False):
        layers = [self.files, *upper_layers]
        names = [f"layer-{i}/layer.tar" for i in range(len(layers))]
        entries = {
            "manifest.json": regular(json.dumps([{"Config": "config.json", "Layers": names}]).encode()),
            "config.json": regular(json.dumps(self.config).encode()),
        }
        physical = list(zip(names, layers))
        if reverse_tar_order:
            physical.reverse()
        entries.update((name, regular(tar_bytes(files))) for name, files in physical)
        self.path.write_bytes(tar_bytes(entries))

    def verify(self, **kwargs):
        verify_image(self.path, **({"cuda": "1", "mode": "dev"} | kwargs))

    def test_valid_cuda_and_mode_combinations(self):
        for cuda in ("0", "1"):
            for mode in ("dev", "run"):
                with self.subTest(cuda=cuda, mode=mode):
                    context = copy.deepcopy(self.context)
                    context["mode"] = mode
                    context["attach_ctx"]["flox_env_cuda_detection"] = cuda
                    self.files[CONTEXT] = regular(json.dumps(context).encode())
                    self.write_image()
                    self.verify(cuda=cuda, mode=mode)

    def test_rejects_hardcoded_disabled_gate(self):
        self.context["attach_ctx"]["flox_env_cuda_detection"] = "0"
        self.files[CONTEXT] = regular(json.dumps(self.context).encode())
        self.write_image()
        with self.assertRaisesRegex(InvalidImage, "CUDA gate"):
            self.verify()

    def test_rejects_wrong_activation_mode(self):
        self.write_image()
        with self.assertRaisesRegex(InvalidImage, "Activation mode"):
            self.verify(mode="run")

    def test_rejects_account_links_independently(self):
        original = self.files.copy()
        for name in ("etc/passwd", "etc/group"):
            for kind in (tarfile.SYMTYPE, tarfile.LNKTYPE):
                for target in ("/nix/store/fake-nss/" + name, "../nix/store/fake-nss/" + name):
                    with self.subTest(name=name, kind=kind, target=target):
                        self.files = original.copy()
                        self.files[name] = link(target, kind)
                        self.files["nix/store/fake-nss/" + name] = link("/nix/store/accounts/" + name)
                        self.files["nix/store/accounts/" + name] = original[name]
                        self.write_image()
                        with self.assertRaisesRegex(InvalidImage, name + ": expected a regular file"):
                            self.verify()

    def test_never_reads_context_from_the_host_store(self):
        host_context = Path(self.temporary.name) / "host-context"
        host_context.write_text(json.dumps(self.context))
        self.config["config"]["Entrypoint"][3] = str(host_context)
        del self.files[CONTEXT]
        self.write_image()
        with self.assertRaisesRegex(InvalidImage, "Image is missing.*host-context"):
            self.verify()

    def test_uses_manifest_layer_order_not_physical_tar_order(self):
        bad_context = copy.deepcopy(self.context)
        bad_context["attach_ctx"]["flox_env_cuda_detection"] = "0"
        self.write_image({CONTEXT: regular(json.dumps(bad_context).encode())}, reverse_tar_order=True)
        with self.assertRaisesRegex(InvalidImage, "CUDA gate"):
            self.verify()

    def test_accepts_regular_file_replacing_lower_symlink(self):
        self.files["etc/passwd"] = link("/nix/store/old-passwd")
        self.write_image({"etc/passwd": regular(PASSWD)})
        self.verify()

    def test_rejects_symlink_replacing_lower_regular_file(self):
        self.write_image({"etc/group": link("/nix/store/old-group")})
        with self.assertRaisesRegex(InvalidImage, "etc/group: expected a regular file"):
            self.verify()

    def test_whiteout_cannot_leave_stale_account_evidence(self):
        for name in ("etc/.wh.passwd", ".wh.etc", "etc/.wh..wh..opq", ".wh..wh..opq"):
            with self.subTest(name=name):
                self.write_image({name: regular(b"")})
                with self.assertRaisesRegex(InvalidImage, "Image is missing"):
                    self.verify()

    def test_opaque_whiteout_does_not_hide_its_own_layer(self):
        self.write_image({"etc/passwd": regular(PASSWD), "etc/group": regular(GROUP),
                          "etc/.wh..wh..opq": regular(b"")})
        self.verify()

    def test_rejects_symlinked_etc_directory(self):
        self.files["etc"] = link("/nix/store/etc")
        self.write_image()
        with self.assertRaisesRegex(InvalidImage, "etc: expected a real directory"):
            self.verify()

    def test_rejects_dropped_default_records(self):
        for name, data in (("etc/passwd", PASSWD), ("etc/group", GROUP)):
            with self.subTest(name=name):
                self.write_image({name: regular(data.splitlines(keepends=True)[0])})
                with self.assertRaisesRegex(InvalidImage, name + ": expected"):
                    self.verify()

    def test_nonroot_records_ownership_and_working_directory(self):
        passwd = "flox:x:1234:5678:created by Flox:/var/empty:/bin/sh"
        group = "flox:x:5678:"
        self.config["config"].update(User="1234:5678", WorkingDir="/workspace")
        self.files["etc/passwd"] = regular(PASSWD.replace(b"nobody:", (passwd + "\nnobody:").encode(), 1))
        self.files["etc/group"] = regular(GROUP.replace(b"nobody:", (group + "\nnobody:").encode(), 1))
        self.files.update({"run": directory(0o1770, 1234, 5678),
                           "run/flox": directory(0o1770, 1234, 5678),
                           "workspace": directory(0o755, 1234, 5678)})
        expected = dict(user="1234:5678", passwd_lines=[passwd], group_lines=[group],
                        uid=1234, gid=5678, working_dir="/workspace")
        self.write_image()
        self.verify(**expected)
        self.write_image({"etc/group": regular(GROUP)})
        with self.assertRaisesRegex(InvalidImage, "etc/group: expected"):
            self.verify(**expected)
        self.write_image({"run": directory(0o1770)})
        with self.assertRaisesRegex(InvalidImage, "run: expected mode/uid/gid"):
            self.verify(**expected)

    def test_rejects_unreadable_accounts(self):
        self.write_image({"etc/passwd": regular(PASSWD, mode=0o400)})
        with self.assertRaisesRegex(InvalidImage, "not readable"):
            self.verify()

    def test_rejects_wrong_entrypoint_environment(self):
        self.config["config"]["Entrypoint"][0] = "/nix/store/other/libexec/flox-activations"
        self.write_image()
        with self.assertRaisesRegex(InvalidImage, "different environments"):
            self.verify()


if __name__ == "__main__":
    unittest.main(verbosity=2)
