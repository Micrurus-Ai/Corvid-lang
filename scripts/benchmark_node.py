"""Launch a local Node baseline for paired browser benchmarks."""
from contextlib import contextmanager
import socket
import subprocess
import time

from verify_reading_list import read


@contextmanager
def node_server(directory, script, settings, ready):
    with socket.socket() as sock:
        sock.bind(("127.0.0.1", 0))
        port = sock.getsockname()[1]
    proc = subprocess.Popen(["node", script, str(port), settings], cwd=directory)
    url = f"http://127.0.0.1:{port}"
    try:
        for _ in range(150):
            if proc.poll() is not None:
                raise RuntimeError("TypeScript server exited")
            try:
                read(url + ready)
                break
            except OSError:
                time.sleep(.05)
        else:
            raise TimeoutError(url)
        yield url
    finally:
        proc.terminate()
        proc.wait(timeout=10)
