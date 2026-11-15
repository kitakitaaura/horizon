"""Serves web/ on localhost with the headers shared-memory threads need (COOP and COEP).

Usage: python3 web/serve.py [PORT] [--resources FOLDER]

--resources serves a prepared folder (horizon-prepare's output) at /resources/, with the list of
its files at /resources/index.json, for the page's `?fetch` import while developing: only on
localhost, to your own browser.
"""
import http.server
import json
import os
import pathlib
import sys

ROOT = pathlib.Path(__file__).resolve().parent


RESOURCES = None


class Handler(http.server.SimpleHTTPRequestHandler):
    def __init__(self, *args, **kwargs):
        super().__init__(*args, directory=str(ROOT), **kwargs)

    def translate_path(self, path):
        if RESOURCES is not None and path.split("?")[0].startswith("/resources/"):
            rel = path.split("?")[0][len("/resources/"):]
            target = (RESOURCES / rel).resolve()
            if RESOURCES == target or RESOURCES in target.parents:
                return str(target)
            return str(RESOURCES / "__outside__")
        return super().translate_path(path)

    def do_GET(self):
        if RESOURCES is not None and self.path.split("?")[0] == "/resources/index.json":
            files = sorted(
                str(p.relative_to(RESOURCES)).replace(os.sep, "/")
                for p in RESOURCES.rglob("*")
                if p.is_file()
            )
            body = json.dumps(files).encode()
            self.send_response(200)
            self.send_header("Content-Type", "application/json")
            self.send_header("Content-Length", str(len(body)))
            self.end_headers()
            self.wfile.write(body)
            return
        super().do_GET()

    def log_message(self, format, *args):
        if not self.path.startswith("/resources/"):
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
    port = int(args[0]) if args else 8765
    http.server.ThreadingHTTPServer(("127.0.0.1", port), Handler).serve_forever()
