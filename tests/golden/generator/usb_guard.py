"""Make real USB unreachable for the golden generator. Import this FIRST.

Every generator entry point (generate.py, worker.py) calls `install()` before it
imports pyusb, galvoplotter or any OpenLaser prototype (OM) code. After
`install()`:

1. A CPython audit hook refuses every `ctypes.dlopen` of a real library
   (anything except `dlopen(NULL)`, which `import ctypes` itself performs).
   pyusb reaches libusb-1.0, libusb-0.1 and OpenUSB only through
   `ctypes.CDLL`, so no USB library can be loaded into this process at all.
   Audit hooks cannot be removed once added.
2. `usb.core.find`, `usb.core.show_devices` and every pyusb backend's
   `get_backend` are replaced by functions that raise.
3. galvoplotter's `USBConnection` (in both `galvo.usb_connection` and
   `galvo.controller`, which imported it by name) is replaced by a class that
   raises when constructed. `galvo.mock_connection.MockConnection` is left
   alone: it never touches USB, and OM only uses it in OPENMOPA_MOCK mode.
4. `self_test()` proves each blocked path raises, and `assert_clean()` (run at
   the end of every worker) proves that no USB library was ever loaded and
   that nothing except the self-test tried.

Run `python usb_guard.py` (with the OM venv's python) to see the self-test.
"""

from __future__ import annotations

import sys

GUARD_VERSION = "1"


class UsbBlockedError(RuntimeError):
    """Raised by every blocked USB path."""


_STATE = {"installed": False, "attempts": [], "self_test": False}


def _record(what: str) -> None:
    _STATE["attempts"].append({"what": what, "during_self_test": bool(_STATE["self_test"])})


def _audit_hook(event: str, args: tuple) -> None:
    if event == "ctypes.dlopen":
        name = args[0] if args else None
        if name is None:  # dlopen(NULL): the running process itself (`import ctypes`)
            return
        _record(f"ctypes.dlopen({name!r})")
        raise UsbBlockedError(f"golden generator: loading native library {name!r} is blocked (no USB)")


def _blocked(label: str):
    def blocked(*args, **kwargs):
        _record(label)
        raise UsbBlockedError(f"golden generator: {label} is blocked (no USB)")

    blocked.__name__ = f"blocked_{label.replace('.', '_')}"
    return blocked


class BlockedUSBConnection:
    """Stands in for galvo.usb_connection.USBConnection: constructing it raises."""

    def __init__(self, *args, **kwargs):
        _record("galvo USBConnection()")
        raise UsbBlockedError("golden generator: galvoplotter USBConnection is blocked (no USB)")


def install() -> None:
    """Block every route to real USB. Idempotent. Call before any other import."""
    if _STATE["installed"]:
        return
    sys.addaudithook(_audit_hook)

    import usb.core  # noqa: E402  (pyusb; importing it loads no native library)
    import usb.backend.libusb0 as libusb0  # noqa: E402
    import usb.backend.libusb1 as libusb1  # noqa: E402
    import usb.backend.openusb as openusb  # noqa: E402

    usb.core.find = _blocked("usb.core.find")
    if hasattr(usb.core, "show_devices"):
        usb.core.show_devices = _blocked("usb.core.show_devices")
    libusb0.get_backend = _blocked("usb.backend.libusb0.get_backend")
    libusb1.get_backend = _blocked("usb.backend.libusb1.get_backend")
    openusb.get_backend = _blocked("usb.backend.openusb.get_backend")

    import galvo.usb_connection as gp_usb  # noqa: E402
    import galvo.controller as gp_controller  # noqa: E402

    gp_usb.USBConnection = BlockedUSBConnection
    gp_controller.USBConnection = BlockedUSBConnection

    try:  # OM's bundled-libusb fallback; patched for completeness
        import libusb_package  # noqa: E402

        for name in ("find", "find_library", "get_libusb1_backend", "get_library_path"):
            if hasattr(libusb_package, name):
                setattr(libusb_package, name, _blocked(f"libusb_package.{name}"))
    except ImportError:
        pass

    _STATE["installed"] = True


def self_test() -> list[str]:
    """Try every blocked path; each must raise UsbBlockedError. Returns the paths proven."""
    import ctypes

    import galvo.controller as gp_controller
    import galvo.usb_connection as gp_usb
    import usb.backend.libusb1 as libusb1
    import usb.core

    probes = {
        "usb.core.find(idVendor=0x9588, idProduct=0x9899)": lambda: usb.core.find(idVendor=0x9588, idProduct=0x9899),
        "usb.backend.libusb1.get_backend()": lambda: libusb1.get_backend(),
        "galvo.controller.USBConnection()": lambda: gp_controller.USBConnection(print),
        "galvo.usb_connection.USBConnection()": lambda: gp_usb.USBConnection(print),
        "ctypes.CDLL('libusb-1.0.dylib')": lambda: ctypes.CDLL("libusb-1.0.dylib"),
        "ctypes.CDLL('libusb-1.0.so.0')": lambda: ctypes.CDLL("libusb-1.0.so.0"),
    }
    proven: list[str] = []
    _STATE["self_test"] = True
    try:
        for label, probe in probes.items():
            try:
                probe()
            except UsbBlockedError:
                proven.append(label)
                continue
            raise AssertionError(f"USB guard self-test FAILED: {label} did not raise")
    finally:
        _STATE["self_test"] = False
    return proven


def assert_clean() -> dict:
    """Prove no USB library was loaded and nothing but the self-test tried to reach USB."""
    import usb.backend.libusb0 as libusb0
    import usb.backend.libusb1 as libusb1
    import usb.backend.openusb as openusb

    loaded = {
        "libusb1._lib": getattr(libusb1, "_lib", None),
        "libusb0._lib": getattr(libusb0, "_lib", None),
        "openusb._lib": getattr(openusb, "_lib", None),
    }
    if any(value is not None for value in loaded.values()):
        raise AssertionError(f"a USB library was loaded: {loaded}")
    unexpected = [a for a in _STATE["attempts"] if not a["during_self_test"]]
    if unexpected:
        raise AssertionError(f"blocked USB attempts outside the self-test: {unexpected}")
    return {
        "guard_version": GUARD_VERSION,
        "usb_libraries_loaded": 0,
        "blocked_attempts_outside_self_test": 0,
        "self_test_probes": sum(1 for a in _STATE["attempts"] if a["during_self_test"]),
    }


if __name__ == "__main__":
    install()
    for line in self_test():
        print(f"blocked: {line}")
    print(assert_clean())
