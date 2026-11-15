"""An isolated loopback probe. It never contacts a real RPC or other live endpoint."""
import errno
import socket
import subprocess

assert {name for _, name in socket.if_nameindex()} == {"lo"}
assert not subprocess.check_output(["ip", "route", "show", "default"]).strip()
with socket.socket() as listener:
    listener.bind(("127.0.0.1", 0))
    listener.listen(1)
    with socket.create_connection(listener.getsockname(), timeout=2) as client:
        peer, address = listener.accept()
        with peer:
            assert address[0] == "127.0.0.1"
            peer.sendall(b"public-ci-fixture")
            assert client.recv(64) == b"public-ci-fixture"
with socket.socket() as blocked:
    blocked.settimeout(1)
    try:
        blocked.connect(("203.0.113.1", 443))
    except OSError as error:
        assert error.errno in {errno.ENETUNREACH, errno.EHOSTUNREACH}
    else:
        raise AssertionError("External route unexpectedly available")
print("Loopback works; the isolated namespace has no external route")
