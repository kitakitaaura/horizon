"""Serves web/ on localhost with the headers shared-memory threads need (COOP and COEP).

Usage: python3 web/serve.py [PORT] [--resources FOLDER] [--minecraft FOLDER] [--mount NAME=FOLDER]...

--resources serves a prepared folder (horizon-prepare's output) at /resources/, with the list of
its files at /resources/index.json, for the page's `?fetch` import while developing.
--minecraft serves your Minecraft folder at /minecraft-folder/, for `?fetch-minecraft`, which
reads it as the folder picker would.
--mount serves FOLDER at /mnt/NAME/, with its files and sizes at /mnt/NAME/.index.json, for the Java
runtime's worker (web/jvm/) while developing: a JDK image, a run folder.
All of them only on localhost, to your own browser.
"""
import http.server
import json
import os
import pathlib
import sys

ROOT = pathlib.Path(__file__).resolve().parent


RESOURCES = None
MINECRAFT = None
MOUNTS = {}


def mounted(path):
    """The file a /resources/ or /minecraft-folder/ path names, or None for another path."""
    path = path.split("?")[0]
    named = [(f"/mnt/{name}/", folder) for name, folder in MOUNTS.items()]
    for prefix, folder in [("/resources/", RESOURCES), ("/minecraft-folder/", MINECRAFT), *named]:
        if folder is not None and path.startswith(prefix):
            from urllib.parse import unquote
            target = (folder / unquote(path[len(prefix):])).resolve()
            if folder == target or folder in target.parents:
                return str(target)
            return str(folder / "__outside__")
    return None


class Handler(http.server.SimpleHTTPRequestHandler):
    def __init__(self, *args, **kwargs):
        super().__init__(*args, directory=str(ROOT), **kwargs)

    def translate_path(self, path):
        return mounted(path) or super().translate_path(path)

    def do_GET(self):
        path = self.path.split("?")[0]
        for name, folder in MOUNTS.items():
            if path == f"/mnt/{name}/.index.json":
                files = sorted(
                    [str(p.relative_to(folder)).replace(os.sep, "/"), p.stat().st_size]
                    for p in folder.rglob("*")
                    if p.is_file()
                )
                return self.send_json(files)
        if RESOURCES is not None and self.path.split("?")[0] == "/resources/index.json":
            files = sorted(
                str(p.relative_to(RESOURCES)).replace(os.sep, "/")
                for p in RESOURCES.rglob("*")
                if p.is_file()
            )
            return self.send_json(files)
        super().do_GET()

    def send_json(self, value):
        body = json.dumps(value).encode()
        self.send_response(200)
        self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(body)))
        self.end_headers()
        self.wfile.write(body)

    def log_message(self, format, *args):
        if not self.path.startswith(("/resources/", "/minecraft-folder/", "/mnt/")):
            super().log_message(format, *args)

    def end_headers(self):
        self.send_header("Cross-Origin-Opener-Policy", "same-origin")
        self.send_header("Cross-Origin-Embedder-Policy", "require-corp")
        self.send_header("Cache-Control", "no-store")
        super().end_headers()


http.server.SimpleHTTPRequestHandler.extensions_map[".wasm"] = "application/wasm"
http.server.SimpleHTTPRequestHandler.extensions_map[".js"] = "text/javascript"

if __name__ == "__main__":
    args = sys.argv[1:]
    if "--resources" in args:
        i = args.index("--resources")
        RESOURCES = pathlib.Path(args[i + 1]).expanduser().resolve()
        del args[i : i + 2]
    if "--minecraft" in args:
        i = args.index("--minecraft")
        MINECRAFT = pathlib.Path(args[i + 1]).expanduser().resolve()
        del args[i : i + 2]
    while "--mount" in args:
        i = args.index("--mount")
        name, folder = args[i + 1].split("=", 1)
        MOUNTS[name] = pathlib.Path(folder).expanduser().resolve()
        del args[i : i + 2]
    port = int(args[0]) if args else 8765
    # The page imports files 16 at a time: the default backlog of 5 drops connections (a timeout).
    http.server.ThreadingHTTPServer.request_queue_size = 128
    http.server.ThreadingHTTPServer(("127.0.0.1", port), Handler).serve_forever()
